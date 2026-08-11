use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::Value;
use walkdir::WalkDir;

use crate::project::SessionConfig;

// 编译时把 Groovy 代理嵌入 EXE；运行时再写入临时目录交给 DVCfgCmd。
const DAEMON_SCRIPT: &str = include_str!("../../assets/LGKVectorDaemon.dvgroovy");
const DAVINCI_START_TIMEOUT: Duration = Duration::from_secs(45);
const DAVINCI_OPERATION_TIMEOUT: Duration = Duration::from_secs(120);
const DAVINCI_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);
const DAVINCI_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);

// 一个已启动的 DVCfgCmd/DaVinci 子进程及其本机通信信息。
pub struct DaVinciClient {
    child: Child,
    port: u16,
    runtime_dir: PathBuf,
}

impl DaVinciClient {
    pub fn start(config: &SessionConfig) -> Result<Self> {
        // 用户可显式选择命令程序；否则保持旧行为，从 tool_path 自动发现。
        let dvcfg = resolve_davinci_command(config)?;
        let dpa = config.dpa_file()?;
        // 每个 DaVinci 会话使用独立临时目录，避免并发会话共用 Groovy 或日志。
        let runtime_dir = std::env::temp_dir().join(format!(
            "lgk-vector-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&runtime_dir)
            .with_context(|| format!("create runtime dir: {}", runtime_dir.display()))?;
        let script_path = runtime_dir.join("LGKVectorDaemon.dvgroovy");
        fs::write(&script_path, DAEMON_SCRIPT)
            .with_context(|| format!("write script: {}", script_path.display()))?;
        let stdout_log = runtime_dir.join("DVCfgCmd.stdout.log");
        let stderr_log = runtime_dir.join("DVCfgCmd.stderr.log");

        // DVCfgCmd 负责打开真实 DPA；脚本目录只包含本工具临时写出的代理。
        let mut command = Command::new(&dvcfg);
        command
            .current_dir(&config.project_path)
            .arg("--project")
            .arg(&dpa)
            .arg("--scriptLocations")
            .arg(&runtime_dir)
            .arg("--scriptTask")
            .arg("LGKVectorDaemon")
            .arg("--ignoreUserScriptLocations")
            .arg("--verbose")
            .arg("ERROR")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_window(&mut command);
        let mut child = command
            .spawn()
            .with_context(|| format!("start {}", dvcfg.display()))?;
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_owned_child(&mut child);
                bail!("DVCfgCmd stdout pipe unavailable");
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_owned_child(&mut child);
                bail!("DVCfgCmd stderr pipe unavailable");
            }
        };
        // Groovy 完成加载后会在 stdout 打印随机端口；在收到它之前不能发送命令。
        let (port_sender, port_receiver) = mpsc::sync_channel(1);
        spawn_stdout_reader(stdout, stdout_log, port_sender);
        spawn_log_reader(stderr, stderr_log);

        let port = match port_receiver.recv_timeout(DAVINCI_START_TIMEOUT) {
            Ok(port) => port,
            Err(error) => {
                let reason = match child.try_wait() {
                    Ok(Some(status)) => format!("DaVinci daemon exited with {status}"),
                    Ok(None) => {
                        terminate_owned_child(&mut child);
                        format!("DaVinci daemon did not become ready: {error}; process was stopped")
                    }
                    Err(wait_error) => {
                        terminate_owned_child(&mut child);
                        format!(
                            "DaVinci daemon readiness failed: {error}; process status failed: {wait_error}; process was stopped"
                        )
                    }
                };
                bail!(startup_failure_detail(&runtime_dir, &reason));
            }
        };
        Ok(Self {
            child,
            port,
            runtime_dir,
        })
    }

    pub fn list_errors(&mut self, module: &str) -> Result<Value> {
        let lines = self.send(&format!("LIST|{module}"))?;
        let start = lines
            .iter()
            .position(|line| line == "ECUC_JSON_BEGIN")
            .ok_or_else(|| anyhow::anyhow!("no JSON_BEGIN in response"))?;
        let end = lines
            .iter()
            .position(|line| line == "ECUC_JSON_END")
            .ok_or_else(|| anyhow::anyhow!("no JSON_END in response"))?;
        if end <= start {
            bail!("invalid JSON markers in daemon response");
        }
        let raw = lines[start + 1..end].join("\n");
        serde_json::from_str(&raw).context("parse error list")
    }

    pub fn solve_errors(&mut self, module: &str, targets: Option<&str>) -> Result<String> {
        let command = match targets {
            Some(targets) if !targets.trim().is_empty() => {
                format!("SOLVE|{module}|{}", targets.trim())
            }
            _ => format!("SOLVE|{module}"),
        };
        Ok(self
            .send(&command)?
            .into_iter()
            .filter(|line| line != "ECUC_END")
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub fn generate(&mut self, module: &str, definition_ref: Option<&str>) -> Result<String> {
        // 生成协议把真实 definition_ref 一并传给 Groovy。
        let command = generation_command(module, definition_ref)?;
        Ok(self
            .send(&command)?
            .into_iter()
            .filter(|line| line != "ECUC_END")
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub fn shutdown(mut self) -> Result<()> {
        // 正常关闭优先：让 DaVinci 自己保存/释放会话，而不是强制结束子进程。
        let _ = self.send("SHUTDOWN")?;
        let deadline = Instant::now() + DAVINCI_SHUTDOWN_TIMEOUT;
        while Instant::now() < deadline {
            if self.child.try_wait()?.is_some() {
                cleanup_runtime_dir(&self.runtime_dir);
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
        terminate_owned_child(&mut self.child);
        bail!(
            "DaVinci daemon did not exit after SHUTDOWN and was force-stopped; runtime preserved at {}",
            self.runtime_dir.display()
        )
    }

    fn send(&mut self, command: &str) -> Result<Vec<String>> {
        let result = self.send_inner(command);
        if result.is_err() {
            terminate_owned_child(&mut self.child);
        }
        result
    }

    fn send_inner(&self, command: &str) -> Result<Vec<String>> {
        // 第二个端口同样只使用 127.0.0.1；它是 Rust 与 DaVinci 内 Groovy 的私有通道。
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port);
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))
            .with_context(|| format!("connect to DaVinci daemon at {address}"))?;
        stream.set_read_timeout(Some(DAVINCI_OPERATION_TIMEOUT))?;
        writeln!(stream, "{command}")?;
        stream.flush()?;

        let mut lines = Vec::new();
        for line in BufReader::new(stream).lines() {
            let line = line.context(
                "failed to read the DaVinci response within the 120-second operation timeout",
            )?;
            let done = line == "ECUC_END";
            lines.push(line);
            if done {
                fail_if_da_vinci_reported_failure(&lines)?;
                return Ok(lines);
            }
        }
        bail!("DaVinci daemon closed the connection without ECUC_END")
    }
}

fn fail_if_da_vinci_reported_failure(lines: &[String]) -> Result<()> {
    if let Some(message) = lines
        .iter()
        .find_map(|line| line.strip_prefix("FAIL:"))
        .map(str::trim)
    {
        bail!("DaVinci daemon reported failure: {message}");
    }
    Ok(())
}

impl Drop for DaVinciClient {
    fn drop(&mut self) {
        // std::process::Child does not kill on drop. Always reap the process we
        // own so an error path cannot leave DVCfgCmd consuming memory.
        terminate_owned_child(&mut self.child);
    }
}

fn generation_command(module: &str, definition_ref: Option<&str>) -> Result<String> {
    // all 不需要筛选生成器；单模块必须带真实模块定义路径。
    if module.eq_ignore_ascii_case("all") {
        return Ok("GEN|all".to_string());
    }
    let definition_ref = definition_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("module definition ref is required for generation"))?;
    if module.contains('|') || definition_ref.contains('|') {
        bail!("module and definition ref must not contain '|'");
    }
    Ok(format!("GEN|{module}|{definition_ref}"))
}

pub(crate) fn resolve_davinci_command(config: &SessionConfig) -> Result<PathBuf> {
    match &config.davinci_command_path {
        Some(path) => Ok(path.clone()),
        None => find_dvcfg(&config.tool_path),
    }
}

fn find_dvcfg(tool_path: &Path) -> Result<PathBuf> {
    let deadline = Instant::now() + DAVINCI_DISCOVERY_TIMEOUT;
    let mut candidates = Vec::new();
    for entry in WalkDir::new(tool_path).follow_links(false) {
        if Instant::now() >= deadline {
            bail!(
                "DVCfgCmd.exe discovery exceeded 2 seconds under {}; set davinci_command_path explicitly",
                tool_path.display()
            );
        }
        let Ok(entry) = entry else {
            continue;
        };
        if entry.file_type().is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("DVCfgCmd.exe")
        {
            candidates.push(entry.into_path());
        }
    }
    candidates.sort();
    match candidates.as_slice() {
        [] => bail!(
            "DVCfgCmd.exe not found under tool_path: {}",
            tool_path.display()
        ),
        [path] => Ok(path.clone()),
        _ => bail!(
            "found multiple DVCfgCmd.exe under tool_path: {}",
            candidates
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn terminate_owned_child(child: &mut Child) {
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }
    terminate_owned_process_tree(child);
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(windows)]
fn terminate_owned_process_tree(child: &Child) {
    // DVCfgCmd may create helper JVM processes. taskkill receives the exact PID
    // returned by Command::spawn and /T limits cleanup to that owned tree.
    let mut command = Command::new("taskkill");
    command
        .arg("/PID")
        .arg(child.id().to_string())
        .arg("/T")
        .arg("/F")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut command);
    let _ = command.status();
}

#[cfg(not(windows))]
fn terminate_owned_process_tree(_child: &Child) {}

fn spawn_stdout_reader(
    stdout: impl std::io::Read + Send + 'static,
    log_path: PathBuf,
    port_sender: mpsc::SyncSender<u16>,
) {
    thread::spawn(move || {
        let mut log = create_log(&log_path);
        let mut sent = false;
        for line in BufReader::new(stdout).lines().map_while(|line| line.ok()) {
            if let Some(log) = log.as_mut() {
                let _ = writeln!(log, "{line}");
            }
            if !sent {
                if let Some(raw_port) = line.trim().strip_prefix("LGK_VECTOR_READY:") {
                    if let Ok(port) = raw_port.parse::<u16>() {
                        let _ = port_sender.send(port);
                        sent = true;
                    }
                }
            }
        }
    });
}

fn spawn_log_reader(reader: impl std::io::Read + Send + 'static, log_path: PathBuf) {
    thread::spawn(move || {
        let mut log = create_log(&log_path);
        for line in BufReader::new(reader).lines().map_while(|line| line.ok()) {
            if let Some(log) = log.as_mut() {
                let _ = writeln!(log, "{line}");
            }
        }
    });
}

fn create_log(path: &Path) -> Option<File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn startup_failure_detail(runtime_dir: &Path, reason: &str) -> String {
    let stdout_log = runtime_dir.join("DVCfgCmd.stdout.log");
    let stderr_log = runtime_dir.join("DVCfgCmd.stderr.log");
    let combined = [&stdout_log, &stderr_log]
        .into_iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let lower = combined.to_ascii_lowercase();
    let detail = if lower.contains(".dpa") && lower.contains("locked by another application") {
        "The .dpa project is locked by another application. Close the DaVinci GUI before a DaVinci-backed request, or use inspect_ecuc_containers for read-only ECUC inspection."
            .to_string()
    } else {
        combined
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let detail = if detail.is_empty() {
        "No diagnostic text was written by DVCfgCmd.".to_string()
    } else {
        detail
    };
    format!(
        "{reason}. {detail} Logs and the generated bridge script were preserved at {}",
        runtime_dir.display()
    )
}

fn cleanup_runtime_dir(path: &Path) {
    let temp = std::env::temp_dir();
    if path.starts_with(&temp)
        && path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("lgk-vector-"))
    {
        fs::remove_dir_all(path).ok();
    }
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        fail_if_da_vinci_reported_failure, generation_command, startup_failure_detail,
        DAVINCI_DISCOVERY_TIMEOUT, DAVINCI_OPERATION_TIMEOUT, DAVINCI_SHUTDOWN_TIMEOUT,
        DAVINCI_START_TIMEOUT,
    };

    #[test]
    fn ordinary_operation_budget_does_not_exceed_three_minutes() {
        assert!(
            DAVINCI_START_TIMEOUT + DAVINCI_OPERATION_TIMEOUT + DAVINCI_SHUTDOWN_TIMEOUT
                <= std::time::Duration::from_secs(180)
        );
        assert!(DAVINCI_DISCOVERY_TIMEOUT <= std::time::Duration::from_secs(2));
    }

    #[test]
    fn generation_uses_actual_vendor_definition_ref() {
        assert_eq!(
            generation_command("Nm", Some("/VendorStack/Nm")).expect("command"),
            "GEN|Nm|/VendorStack/Nm"
        );
        assert_eq!(generation_command("all", None).expect("all"), "GEN|all");
    }

    #[test]
    fn daemon_failure_line_is_never_reported_as_success() {
        let lines = vec![
            "FAIL: generator rejected the module".to_string(),
            "ECUC_END".to_string(),
        ];
        let error = fail_if_da_vinci_reported_failure(&lines)
            .expect_err("FAIL response must become an error");
        assert!(error.to_string().contains("generator rejected the module"));
    }

    #[test]
    fn startup_failure_explains_dpa_lock_and_preserves_log_location() {
        let runtime = tempdir().expect("runtime");
        fs::write(
            runtime.path().join("DVCfgCmd.stderr.log"),
            "The project file (.dpa) is locked by another application",
        )
        .expect("log");

        let message = startup_failure_detail(runtime.path(), "daemon exited");
        assert!(message.contains("locked by another application"));
        assert!(message.contains("inspect_ecuc_containers"));
        assert!(message.contains(&runtime.path().display().to_string()));
    }
}
