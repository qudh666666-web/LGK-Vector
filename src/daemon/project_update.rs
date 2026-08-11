use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use walkdir::WalkDir;

use crate::daemon::client::resolve_davinci_command;
use crate::project::SessionConfig;

const PROJECT_UPDATE_TIMEOUT: Duration = Duration::from_secs(165);

#[derive(Debug)]
struct ImportPlan {
    source: PathBuf,
    destination: PathBuf,
}

/// Exact in-memory snapshot of the Cfg tree before DaVinci Project Update.
/// It is intentionally project-wide because the converter can touch DPA,
/// ECUC and System Description files before it reports a DBC error.
struct ProjectSnapshot {
    root: PathBuf,
    directories: BTreeSet<PathBuf>,
    files: Vec<(PathBuf, Vec<u8>)>,
}

impl ProjectSnapshot {
    fn capture(root: &Path) -> Result<Self> {
        let mut directories = BTreeSet::new();
        let mut files = Vec::new();
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry.with_context(|| format!("snapshot project: {}", root.display()))?;
            let relative = entry
                .path()
                .strip_prefix(root)
                .context("snapshot path escaped project root")?
                .to_path_buf();
            if relative.as_os_str().is_empty() {
                continue;
            }
            if entry.file_type().is_dir() {
                directories.insert(relative);
            } else if entry.file_type().is_file() {
                files.push((
                    relative,
                    fs::read(entry.path()).with_context(|| {
                        format!("snapshot project file: {}", entry.path().display())
                    })?,
                ));
            } else {
                bail!(
                    "Project Update transaction does not support links or special files: {}",
                    entry.path().display()
                );
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            directories,
            files,
        })
    }

    fn restore(&self) -> Result<()> {
        let original_files = self
            .files
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<BTreeSet<_>>();
        let mut current_directories = Vec::new();
        for entry in WalkDir::new(&self.root).follow_links(false) {
            let entry = entry.with_context(|| {
                format!("inspect failed Project Update: {}", self.root.display())
            })?;
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .context("restore path escaped project root")?
                .to_path_buf();
            if relative.as_os_str().is_empty() {
                continue;
            }
            if entry.file_type().is_file() {
                if !original_files.contains(&relative) {
                    fs::remove_file(entry.path()).with_context(|| {
                        format!(
                            "remove file added by failed Project Update: {}",
                            entry.path().display()
                        )
                    })?;
                }
            } else if entry.file_type().is_dir() {
                current_directories.push(relative);
            }
        }

        for directory in &self.directories {
            fs::create_dir_all(self.root.join(directory))
                .with_context(|| format!("restore project directory: {}", directory.display()))?;
        }
        for (relative, bytes) in &self.files {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, bytes)
                .with_context(|| format!("restore project file: {}", path.display()))?;
        }

        current_directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for relative in current_directories {
            if !self.directories.contains(&relative) {
                let path = self.root.join(relative);
                fs::remove_dir(&path).with_context(|| {
                    format!(
                        "remove directory added by failed Project Update: {}",
                        path.display()
                    )
                })?;
            }
        }
        Ok(())
    }
}

/// Validate a Project Update request without changing the DBC or launching DaVinci.
pub fn validate(config: &SessionConfig, request: &Value) -> Result<()> {
    let _ = prepare(config, request)?;
    Ok(())
}

/// Run DaVinci's supported Project Update workflow. `import_dbc` optionally
/// replaces one DPA-registered DBC first; a failed update restores the complete
/// project tree because DaVinci can touch ECUC before reporting converter errors.
pub fn execute(config: &SessionConfig, request: &Value) -> Result<Value> {
    let import = prepare(config, request)?;
    let snapshot = ProjectSnapshot::capture(&config.project_path)?;

    let result = (|| {
        if let Some(plan) = &import {
            let source_bytes = fs::read(&plan.source)
                .with_context(|| format!("read source DBC: {}", plan.source.display()))?;
            fs::write(&plan.destination, source_bytes).with_context(|| {
                format!("replace registered DBC: {}", plan.destination.display())
            })?;
        }
        run_project_update(config)
    })();

    let mut output = match result {
        Ok(output) => output,
        Err(error) => {
            snapshot.restore().with_context(|| {
                format!(
                    "Project Update failed ({error:#}); project transaction rollback also failed"
                )
            })?;
            bail!("{error:#}; the complete project tree was restored to its pre-update state");
        }
    };
    if let Value::Object(object) = &mut output {
        object.insert(
            "operation".to_string(),
            Value::String(
                if import.is_some() {
                    "import_dbc"
                } else {
                    "update_project"
                }
                .to_string(),
            ),
        );
        object.insert(
            "source_dbc".to_string(),
            import
                .as_ref()
                .map(|plan| Value::String(plan.source.display().to_string()))
                .unwrap_or(Value::Null),
        );
        object.insert(
            "registered_dbc".to_string(),
            import
                .as_ref()
                .map(|plan| Value::String(plan.destination.display().to_string()))
                .unwrap_or(Value::Null),
        );
    }
    Ok(output)
}

fn prepare(config: &SessionConfig, request: &Value) -> Result<Option<ImportPlan>> {
    let func = request
        .get("func")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("func is required"))?;
    config.dpa_file()?;
    resolve_davinci_command(config)?;

    if func == "update_project" {
        if request.get("source").is_some() || request.get("registered_path").is_some() {
            bail!("update_project does not accept source or registered_path; use import_dbc");
        }
        return Ok(None);
    }
    if func != "import_dbc" {
        bail!("unsupported Project Update function: {func}");
    }

    let source = required_path(request, "source")?;
    let source = source
        .canonicalize()
        .with_context(|| format!("source DBC not found: {}", source.display()))?;
    require_dbc(&source, "source")?;

    let registered = request
        .get("registered_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("registered_path is required for import_dbc"))?;
    let registered = Path::new(registered);
    if registered.is_absolute() {
        bail!("registered_path must be relative to project_path");
    }
    // Reuse the project's Windows path normalizer. Plain canonicalize() may
    // return a `\\?\` path while project_path is stored without that prefix,
    // making an in-project file look as if it escaped the project boundary.
    let destination = config
        .ensure_project_file(&config.project_path.join(registered))
        .with_context(|| format!("invalid registered DBC path: {}", registered.display()))?;
    require_dbc(&destination, "registered")?;

    let dpa = config.dpa_file()?;
    let dpa_text = fs::read_to_string(&dpa)
        .with_context(|| format!("read project file: {}", dpa.display()))?;
    let normalized_dpa = dpa_text.replace('\\', "/").to_ascii_lowercase();
    let normalized_registered = registered
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_lowercase();
    let marker = format!("$(dpaprojectfolder)/{normalized_registered}");
    if !normalized_dpa.contains(&marker) {
        bail!(
            "registered_path is not registered as a DPA input: {}",
            registered.display()
        );
    }

    Ok(Some(ImportPlan {
        source,
        destination,
    }))
}

fn run_project_update(config: &SessionConfig) -> Result<Value> {
    let dvcfg = resolve_davinci_command(config)?;
    let dpa = config.dpa_file()?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    // Failure logs live outside the project so a full project rollback cannot
    // erase the evidence. Successful logs are copied into Cfg\Log below.
    let runtime_dir = std::env::temp_dir().join(format!(
        "lgk-vector-project-update-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&runtime_dir)
        .with_context(|| format!("create Project Update runtime: {}", runtime_dir.display()))?;
    let log_path = runtime_dir.join("DVCfgCmd.log");
    let console_path = runtime_dir.join("DVCfgCmd.console.log");
    let console = File::create(&console_path)
        .with_context(|| format!("create console log: {}", console_path.display()))?;
    let console_error = console
        .try_clone()
        .context("clone Project Update console log")?;

    let mut command = Command::new(&dvcfg);
    command
        .current_dir(&config.project_path)
        .arg("--updateProject")
        .arg(&dpa)
        .arg("--ignoreUserScriptLocations")
        .arg("--verbose")
        .arg("INFO")
        .arg("--logfile")
        .arg(&log_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(console))
        .stderr(Stdio::from(console_error));
    hide_window(&mut command);

    let start = Instant::now();
    let mut child = command
        .spawn()
        .with_context(|| format!("start Project Update with {}", dvcfg.display()))?;
    let status = loop {
        if let Some(status) = child.try_wait().context("poll DaVinci Project Update")? {
            break status;
        }
        if start.elapsed() >= PROJECT_UPDATE_TIMEOUT {
            terminate_owned_child(&mut child);
            bail!(
                "DaVinci Project Update exceeded {} seconds and was stopped; logs: {}, {}",
                PROJECT_UPDATE_TIMEOUT.as_secs(),
                log_path.display(),
                console_path.display()
            );
        }
        thread::sleep(Duration::from_millis(100));
    };
    let elapsed_ms = start.elapsed().as_millis();
    let combined = [
        fs::read_to_string(&log_path),
        fs::read_to_string(&console_path),
    ]
    .into_iter()
    .filter_map(Result::ok)
    .collect::<Vec<_>>()
    .join("\n");
    let (converter_errors, converter_warnings, error_lines) = summarize_output(&combined);

    if !status.success() || converter_errors > 0 {
        bail!(
            "DaVinci Project Update failed with exit code {}; elapsed_ms={elapsed_ms}, converter_errors={converter_errors}, converter_warnings={converter_warnings}; logs: {}, {}",
            status.code().unwrap_or(-1),
            log_path.display(),
            console_path.display()
        );
    }

    let project_log_dir = config.project_path.join("Log");
    fs::create_dir_all(&project_log_dir).with_context(|| {
        format!(
            "create Project Update log directory: {}",
            project_log_dir.display()
        )
    })?;
    let saved_log = project_log_dir.join(format!("LGKVectorProjectUpdate-{stamp}.log"));
    let saved_console = project_log_dir.join(format!("LGKVectorProjectUpdate-{stamp}.console.log"));
    fs::copy(&log_path, &saved_log)
        .with_context(|| format!("save Project Update log: {}", saved_log.display()))?;
    fs::copy(&console_path, &saved_console).with_context(|| {
        format!(
            "save Project Update console log: {}",
            saved_console.display()
        )
    })?;
    fs::remove_dir_all(&runtime_dir).ok();

    Ok(json!({
        "status": "updated",
        "exit_code": status.code().unwrap_or(0),
        "elapsed_ms": elapsed_ms,
        "dpa": dpa.display().to_string(),
        "davinci_command": dvcfg.display().to_string(),
        "log_path": saved_log.display().to_string(),
        "console_log_path": saved_console.display().to_string(),
        "converter_errors": converter_errors,
        "converter_warnings": converter_warnings,
        "error_log_lines": error_lines
    }))
}

fn required_path(request: &Value, field: &str) -> Result<PathBuf> {
    let raw = request
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{field} is required"))?;
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        bail!("{field} must be an absolute path");
    }
    Ok(path)
}

fn require_dbc(path: &Path, label: &str) -> Result<()> {
    if !path.is_file()
        || !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("dbc"))
    {
        bail!("{label} path must point to a .dbc file: {}", path.display());
    }
    Ok(())
}

fn summarize_output(raw: &str) -> (u64, u64, usize) {
    // Vector's converter summary uses the plural words "Errors" and
    // "Warnings" even for one item. Requiring the plural avoids mistaking a
    // console timestamp such as `67212 ERROR - ...` for an error count.
    let errors = Regex::new(r"(?im)\b(\d+)\s+Errors\b")
        .expect("error regex")
        .captures_iter(raw)
        .filter_map(|capture| capture[1].parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    let warnings = Regex::new(r"(?im)\b(\d+)\s+Warnings\b")
        .expect("warning regex")
        .captures_iter(raw)
        .filter_map(|capture| capture[1].parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    let error_lines = raw
        .lines()
        .filter(|line| {
            let upper = line.to_ascii_uppercase();
            upper.contains(" ERROR - ") || upper.starts_with("ERROR:")
        })
        .count();
    (errors, warnings, error_lines)
}

fn terminate_owned_child(child: &mut std::process::Child) {
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }
    terminate_owned_process_tree(child);
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(windows)]
fn terminate_owned_process_tree(child: &std::process::Child) {
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
fn terminate_owned_process_tree(_child: &std::process::Child) {}

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

    use serde_json::json;
    use tempfile::tempdir;

    use crate::project::SessionConfig;

    use super::{summarize_output, validate, ProjectSnapshot};

    #[test]
    fn project_update_summary_uses_highest_converter_totals() {
        let raw = "0 Errors, 1 Warnings\n2 Errors 4 Warnings\n67212 ERROR - detail";
        assert_eq!(summarize_output(raw), (2, 4, 1));
    }

    #[test]
    fn import_preflight_accepts_only_a_dpa_registered_destination() {
        let root = tempdir().expect("root");
        let project = root.path().join("Cfg");
        let tool = root.path().join("SIP");
        let dbc_dir = project.join("DBC");
        fs::create_dir_all(&dbc_dir).expect("dbc dir");
        fs::create_dir_all(&tool).expect("tool dir");
        let dpa = project.join("Demo.dpa");
        fs::write(
            &dpa,
            r#"<File>$(DpaProjectFolder)\DBC\Registered.dbc</File>"#,
        )
        .expect("dpa");
        fs::write(dbc_dir.join("Registered.dbc"), "VERSION \"old\"").expect("registered");
        let source = root.path().join("New.dbc");
        fs::write(&source, "VERSION \"new\"").expect("source");
        let dvcfg = tool.join("DVCfgCmd.exe");
        fs::write(&dvcfg, []).expect("dvcfg");
        fs::write(
            project.join("lgk-vector.json"),
            serde_json::to_vec(&json!({
                "tool_path": tool,
                "project_file": "Demo.dpa",
                "davinci_command_path": "DVCfgCmd.exe"
            }))
            .expect("config json"),
        )
        .expect("config");
        let config = SessionConfig::load(&project).expect("session config");

        validate(
            &config,
            &json!({
                "func": "import_dbc",
                "source": source.canonicalize().expect("source"),
                "registered_path": "DBC\\Registered.dbc"
            }),
        )
        .expect("registered DBC should validate");

        let error = validate(
            &config,
            &json!({
                "func": "import_dbc",
                "source": source.canonicalize().expect("source"),
                "registered_path": "DBC\\NotRegistered.dbc"
            }),
        )
        .expect_err("unknown destination must fail");
        assert!(error.to_string().contains("invalid registered DBC path"));
    }

    #[test]
    fn project_snapshot_restores_changed_deleted_and_added_files() {
        let root = tempdir().expect("root");
        let nested = root.path().join("Config");
        fs::create_dir(&nested).expect("nested");
        fs::write(root.path().join("A.arxml"), b"original-a").expect("a");
        fs::write(nested.join("B.dpa"), b"original-b").expect("b");
        let snapshot = ProjectSnapshot::capture(root.path()).expect("snapshot");

        fs::write(root.path().join("A.arxml"), b"changed").expect("change a");
        fs::remove_file(nested.join("B.dpa")).expect("remove b");
        let added_dir = root.path().join("UpdateReport");
        fs::create_dir(&added_dir).expect("added dir");
        fs::write(added_dir.join("report.html"), b"new").expect("added file");

        snapshot.restore().expect("restore");
        assert_eq!(
            fs::read(root.path().join("A.arxml")).expect("a"),
            b"original-a"
        );
        assert_eq!(fs::read(nested.join("B.dpa")).expect("b"), b"original-b");
        assert!(!added_dir.exists());
    }
}
