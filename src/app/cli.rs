use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use rand::RngCore;
use serde_json::Value;

use crate::app::{
    token_file, HostRequest, HostResponse, DEFAULT_HOST_PORT, DEFAULT_HOST_PROBE_PORT,
    HOST_PROTOCOL_VERSION,
};
use crate::project::SessionConfig;

const HOST_REQUEST_TIMEOUT: Duration = Duration::from_secs(175);

// --start-host 使用的入口：创建 Token，必要时启动后台 Host，并等待监听就绪。
pub fn ensure_host() -> Result<()> {
    let token_path = token_file();
    let launch_key = load_or_create_token(&token_path)?;
    match probe_host(&launch_key) {
        Ok(ProbeState::Ready) => return Ok(()),
        Ok(ProbeState::Busy) => {
            bail!("resident host is busy with another request; retry after that request completes")
        }
        Ok(ProbeState::Stopping) => {
            bail!("resident host is shutting down; retry after it exits")
        }
        Err(error) if host_is_listening() => {
            bail!(
                "port {DEFAULT_HOST_PORT} is occupied by an incompatible, stale, or foreign resident host: {error}"
            )
        }
        Err(_) => {}
    }
    start_host(&token_path)?;
    wait_for_host_listener(&launch_key)
}

pub fn run(project_directory: &Path, raw_request: &str) -> Result<String> {
    // 同一安装目录一次只允许一个 CLI 业务请求。它与 Host 的 busy
    // 状态共同阻止第二个写/生成请求排队后在调用方超时之外继续执行。
    let _request_lock = acquire_request_lock()?;
    // CLI 不处理 ECUC 逻辑；它把已校验的工程配置、请求和 Token 一起转发给 Host。
    let shutdown = is_shutdown_request(raw_request)?;
    let token_path = token_file();
    let launch_key = load_or_create_token(&token_path)?;
    let config = if shutdown {
        None
    } else {
        Some(SessionConfig::load(project_directory)?)
    };

    match send_request(config.as_ref(), raw_request, &launch_key, shutdown) {
        Ok(response) => {
            let result = response_text(response);
            if shutdown {
                // The Host acknowledges shutdown just before its listener
                // threads and process finish.  Wait for both ports to close so
                // callers can safely move/delete the project directory or
                // start a matching replacement immediately after this returns.
                wait_for_host_shutdown()?;
            }
            result
        }
        Err(first_error) if host_is_listening() => bail!(
            "port {DEFAULT_HOST_PORT} is occupied by an incompatible, stale, or foreign resident host; stop that process normally before retrying: {first_error}"
        ),
        Err(first_error) if shutdown => Ok(serde_json::json!({
            "status": "not_running",
            "message": first_error.to_string(),
        })
        .to_string()),
        Err(first_error) => bail!(
            "resident host is not running; run --start-host before the request: {first_error}"
        ),
    }
}

pub fn doctor(project_directory: &Path, raw_request: &str) -> Result<String> {
    let config = SessionConfig::load(project_directory)?;
    let functions =
        crate::daemon::commands::CommandDispatcher::validate_batch(&config, raw_request)?;
    // Doctor is intentionally a static preflight. It resolves every path and
    // local prerequisite but does not claim that the proprietary command can
    // start, acquire the DPA, or complete generation.
    let project_file = config.dpa_file()?;
    let davinci_command = crate::daemon::client::resolve_davinci_command(&config)?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "valid": true,
        "preflight": "static",
        "davinci_executed": false,
        "version": env!("CARGO_PKG_VERSION"),
        "project_path": config.project_path,
        "tool_path": config.tool_path,
        "project_file": project_file,
        "davinci_command_path": davinci_command,
        "functions": functions,
    }))?)
}

fn wait_for_host_listener(launch_key: &str) -> Result<()> {
    // 后台进程创建和端口监听是两个时刻，因此轮询直到 Host 真正可连接。
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        match probe_host(launch_key) {
            Ok(ProbeState::Ready) => return Ok(()),
            Ok(ProbeState::Busy) => {
                bail!("new resident host unexpectedly reported busy during startup")
            }
            Ok(ProbeState::Stopping) => {
                bail!("new resident host unexpectedly reported stopping during startup")
            }
            Err(_) => {}
        }
        thread::sleep(Duration::from_millis(100));
    }
    bail!("resident host did not become ready")
}

fn host_is_listening() -> bool {
    port_is_listening(DEFAULT_HOST_PORT)
}

fn port_is_listening(port: u16) -> bool {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_ok()
}

fn wait_for_host_shutdown() -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !port_is_listening(DEFAULT_HOST_PORT) && !port_is_listening(DEFAULT_HOST_PROBE_PORT) {
            // Port closure slightly precedes final Windows process teardown.
            thread::sleep(Duration::from_millis(50));
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }
    bail!(
        "resident host acknowledged shutdown but ports {DEFAULT_HOST_PORT}/{DEFAULT_HOST_PROBE_PORT} did not close"
    )
}

fn is_shutdown_request(raw: &str) -> Result<bool> {
    // shutdown 必须独立发送，避免同一个批次中一半请求已执行、一半被提前关闭。
    let value: Value = serde_json::from_str(raw).context("request is not valid JSON")?;
    match value {
        Value::Object(map) => Ok(map
            .get("func")
            .and_then(Value::as_str)
            .is_some_and(|func| func == "shutdown_host")),
        Value::Array(items) => {
            if items.iter().any(|item| {
                item.get("func")
                    .and_then(Value::as_str)
                    .is_some_and(|func| func == "shutdown_host")
            }) {
                bail!("shutdown_host must be a standalone request");
            }
            Ok(false)
        }
        _ => bail!("request must be a JSON object or array"),
    }
}

fn send_request(
    config: Option<&SessionConfig>,
    raw: &str,
    launch_key: &str,
    shutdown: bool,
) -> Result<HostResponse> {
    send_host_request(
        HostRequest {
            protocol_version: HOST_PROTOCOL_VERSION,
            raw_text: raw.to_string(),
            cfg: config.cloned(),
            launch_key: launch_key.to_string(),
            shutdown,
            probe: false,
        },
        HOST_REQUEST_TIMEOUT,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeState {
    Ready,
    Busy,
    Stopping,
}

fn probe_host(launch_key: &str) -> Result<ProbeState> {
    let response = send_probe_request(
        HostRequest {
            protocol_version: HOST_PROTOCOL_VERSION,
            raw_text: String::new(),
            cfg: None,
            launch_key: launch_key.to_string(),
            shutdown: false,
            probe: true,
        },
        Duration::from_secs(2),
    )?;
    let raw = response_text(response)?;
    let probe: Value = serde_json::from_str(&raw).context("invalid resident host probe")?;
    let version = probe.get("version").and_then(Value::as_str).unwrap_or("");
    let build_id = probe.get("build_id").and_then(Value::as_str).unwrap_or("");
    if version != env!("CARGO_PKG_VERSION") || build_id != crate::app::BUILD_ID {
        bail!(
            "resident host identity mismatch: expected version {} build {}, response={raw}",
            env!("CARGO_PKG_VERSION"),
            crate::app::BUILD_ID
        );
    }
    match probe.get("status").and_then(Value::as_str) {
        Some("ready") => Ok(ProbeState::Ready),
        Some("busy") => Ok(ProbeState::Busy),
        Some("stopping") => Ok(ProbeState::Stopping),
        _ => bail!("resident host returned an unknown activity state: {raw}"),
    }
}

fn send_host_request(request: HostRequest, timeout: Duration) -> Result<HostResponse> {
    // HostRequest 是本机 TCP 上传递的外层信封；raw_text 才是用户原始 JSON 请求。
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_HOST_PORT);
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1))
        .with_context(|| format!("resident host is not running at {address}"))?;
    // On Windows a socket created by connect_timeout can briefly retain its
    // nonblocking state.  Force blocking I/O before the newline-framed JSON
    // exchange so WSAEWOULDBLOCK (10035) is never misreported as a bad Host.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(timeout))?;
    serde_json::to_writer(&mut stream, &request)?;
    writeln!(stream)?;
    stream.flush()?;

    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .context("LGK-Vector request exceeded the 3-minute operation budget")?;
    if response.trim().is_empty() {
        bail!("resident host returned an empty response");
    }
    serde_json::from_str(response.trim()).context("invalid resident host response")
}

fn send_probe_request(request: HostRequest, timeout: Duration) -> Result<HostResponse> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_HOST_PROBE_PORT);
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500))
        .with_context(|| format!("resident probe is not running at {address}"))?;
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(timeout))?;
    serde_json::to_writer(&mut stream, &request)?;
    writeln!(stream)?;
    stream.flush()?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .context("resident probe did not answer within 2 seconds")?;
    if response.trim().is_empty() {
        bail!("resident probe returned an empty response");
    }
    serde_json::from_str(response.trim()).context("invalid resident probe response")
}

fn response_text(response: HostResponse) -> Result<String> {
    if response.protocol_version != HOST_PROTOCOL_VERSION {
        bail!(
            "resident host protocol mismatch: CLI={}, Host={}",
            HOST_PROTOCOL_VERSION,
            response.protocol_version
        );
    }
    if response.exit_code != 0 {
        bail!("{}", response.stderr);
    }
    Ok(response.stdout)
}

fn load_or_create_token(path: &Path) -> Result<String> {
    // Token 保存在 EXE 旁边，CLI 与 Host 通过同一值确认彼此属于同一个本地安装。
    if let Ok(value) = fs::read_to_string(path) {
        let value = value.trim();
        if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(value.to_string());
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("token file has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let token = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            writeln!(file, "{token}")?;
            file.sync_all()?;
            Ok(token)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let value = fs::read_to_string(path)?;
            let value = value.trim();
            if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Ok(value.to_string());
            }
            let mut file = OpenOptions::new().write(true).truncate(true).open(path)?;
            writeln!(file, "{token}")?;
            file.sync_all()?;
            Ok(token)
        }
        Err(error) => Err(error.into()),
    }
}

fn acquire_request_lock() -> Result<fs::File> {
    let path = token_file()
        .parent()
        .ok_or_else(|| anyhow::anyhow!("request lock has no parent directory"))?
        .join("request.lock");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    open_request_lock(&path).with_context(|| {
        "another LGK-Vector request is already running from this installation; wait for it to finish"
    })
}

#[cfg(windows)]
fn open_request_lock(path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
}

#[cfg(not(windows))]
fn open_request_lock(path: &Path) -> std::io::Result<fs::File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

fn start_host(token_path: &Path) -> Result<()> {
    // Host EXE 与 CLI EXE 必须同目录发布，避免调用到另一个版本的后台程序。
    prevent_standard_handle_inheritance();
    let current = std::env::current_exe()?;
    let directory = current
        .parent()
        .ok_or_else(|| anyhow::anyhow!("current executable has no parent directory"))?;
    let executable = host_executable(directory);
    if !executable.is_file() {
        bail!(
            "resident host executable is missing: {}",
            executable.display()
        );
    }
    let mut command = Command::new(&executable);
    command
        .arg("--port")
        .arg(DEFAULT_HOST_PORT.to_string())
        .arg("--token-file")
        .arg(token_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach_resident_host(&mut command);
    command
        .spawn()
        .with_context(|| format!("start {}", executable.display()))?;
    Ok(())
}

fn host_executable(directory: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        directory.join("lgk-vector-host.exe")
    }
    #[cfg(not(windows))]
    {
        directory.join("lgk-vector-host")
    }
}

#[cfg(windows)]
fn detach_resident_host(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    // DETACHED_PROCESS is important when the CLI itself is called from a
    // captured PowerShell pipeline (`@(& lgk-vector.exe --start-host 2>&1)`).
    // Without it, the resident process can keep the caller's console/pipe
    // lifetime alive after the short-lived CLI has exited.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(windows)]
fn prevent_standard_handle_inheritance() {
    use std::ffi::c_void;

    type Handle = *mut c_void;
    const STD_INPUT_HANDLE: u32 = (-10_i32) as u32;
    const STD_OUTPUT_HANDLE: u32 = (-11_i32) as u32;
    const STD_ERROR_HANDLE: u32 = (-12_i32) as u32;
    const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(kind: u32) -> Handle;
        fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
    }

    // PowerShell marks redirection pipe/file handles inheritable.  Rust's
    // child-process launch may otherwise pass those unrelated handles to the
    // resident Host even though its own stdin/stdout/stderr are NUL.  Clearing
    // only the inheritance bit does not close or otherwise alter the CLI's
    // current streams.
    for kind in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: GetStdHandle returns a process-owned pseudo handle and
        // SetHandleInformation is called only to clear HANDLE_FLAG_INHERIT.
        unsafe {
            let handle = GetStdHandle(kind);
            if !handle.is_null() && handle as isize != -1 {
                let _ = SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

#[cfg(not(windows))]
fn detach_resident_host(_command: &mut Command) {}

#[cfg(not(windows))]
fn prevent_standard_handle_inheritance() {}
