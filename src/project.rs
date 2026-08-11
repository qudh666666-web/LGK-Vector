use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

// 磁盘中的工程描述。旧字段名保留仅用于迁移。
#[derive(Debug, Clone, Deserialize)]
struct ConfigFile {
    #[serde(alias = "LGK_project_path", alias = "lgk_project_path")]
    #[serde(default)]
    project_path: Option<PathBuf>,
    #[serde(alias = "LGK_tool_path", alias = "lgk_tool_path")]
    tool_path: PathBuf,
    #[serde(default)]
    project_file: Option<PathBuf>,
    #[serde(default)]
    davinci_command_path: Option<PathBuf>,
}

// 经过路径规范化和边界校验后的会话配置，Host 用它判断请求是否属于同一工程。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionConfig {
    pub project_path: PathBuf,
    pub tool_path: PathBuf,
    #[serde(default)]
    pub project_file: Option<PathBuf>,
    #[serde(default)]
    pub davinci_command_path: Option<PathBuf>,
}

impl SessionConfig {
    pub fn load(project_directory: &Path) -> Result<Self> {
        // 配置文件只能描述它所在的 Cfg 目录，避免请求借配置跨到另一工程。
        let project_directory = canonical_directory(project_directory)
            .context("project path is not an accessible directory")?;
        let config_path = bridge_config_path(&project_directory);
        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("missing or unreadable {}", config_path.display()))?;
        let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
        let parsed: ConfigFile = serde_json::from_str(raw)
            .with_context(|| format!("invalid JSON in {}", config_path.display()))?;

        if !parsed.tool_path.is_absolute() {
            bail!("tool_path must be absolute");
        }

        let configured_project = match parsed.project_path {
            Some(path) => {
                if !path.is_absolute() {
                    bail!("project_path must be absolute when it is specified");
                }
                canonical_directory(&path).context("project_path is not an accessible directory")?
            }
            None => project_directory.clone(),
        };
        if configured_project != project_directory {
            bail!(
                "project_path must equal the directory containing the bridge configuration: configured={}, actual={}",
                configured_project.display(),
                project_directory.display()
            );
        }

        let tool_path = canonical_directory(&parsed.tool_path)
            .context("tool_path is not an accessible directory")?;
        let project_file = parsed
            .project_file
            .map(|path| canonical_project_file(&configured_project, &path))
            .transpose()?;
        let davinci_command_path = parsed
            .davinci_command_path
            .map(|path| canonical_davinci_command(&tool_path, &path))
            .transpose()?;
        Ok(Self {
            project_path: configured_project,
            tool_path,
            project_file,
            davinci_command_path,
        })
    }

    pub fn dpa_file(&self) -> Result<PathBuf> {
        // 多 DPA 工程必须显式选择；只有唯一 DPA 时才允许自动发现。
        if let Some(path) = &self.project_file {
            return Ok(path.clone());
        }
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.project_path)
            .with_context(|| format!("cannot read {}", self.project_path.display()))?
        {
            let path = entry?.path();
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("dpa"))
            {
                files.push(path);
            }
        }
        files.sort();
        match files.as_slice() {
            [] => bail!(
                "no .dpa file found in project_path: {}",
                self.project_path.display()
            ),
            [file] => Ok(file.clone()),
            _ => bail!(
                "multiple .dpa files found in project_path: {}",
                files
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    pub fn ensure_project_file(&self, path: &Path) -> Result<PathBuf> {
        // edit_file 的安全边界：任何写入都必须发生在 project_path 内。
        if !path.is_absolute() {
            bail!("path must be absolute");
        }
        let canonical = path
            .canonicalize()
            .with_context(|| format!("file not found: {}", path.display()))?;
        let canonical = normalize_canonical_path(canonical);
        if !canonical.starts_with(&self.project_path) {
            bail!(
                "refusing to edit a file outside project_path: {}",
                canonical.display()
            );
        }
        if !canonical.is_file() {
            bail!("path is not a file: {}", canonical.display());
        }
        Ok(canonical)
    }
}

fn canonical_project_file(project_path: &Path, path: &Path) -> Result<PathBuf> {
    // 即使用户显式选择 DPA，也不能选择工程目录之外的文件。
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_path.join(path)
    };
    let canonical = normalize_canonical_path(
        path.canonicalize()
            .with_context(|| format!("project_file not found: {}", path.display()))?,
    );
    if !canonical.is_file() {
        bail!("project_file is not a file: {}", canonical.display());
    }
    if !canonical.starts_with(project_path) {
        bail!(
            "project_file must be inside project_path: {}",
            canonical.display()
        );
    }
    if !canonical
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("dpa"))
    {
        bail!("project_file must have a .dpa extension");
    }
    Ok(canonical)
}

fn canonical_davinci_command(tool_path: &Path, path: &Path) -> Result<PathBuf> {
    // 工具程序可以在 project_path 之外，但必须精确指向 DVCfgCmd.exe。
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        tool_path.join(path)
    };
    let canonical = normalize_canonical_path(
        path.canonicalize()
            .with_context(|| format!("davinci_command_path not found: {}", path.display()))?,
    );
    if !canonical.is_file()
        || !canonical
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("DVCfgCmd.exe"))
    {
        bail!(
            "davinci_command_path must point to DVCfgCmd.exe: {}",
            canonical.display()
        );
    }
    Ok(canonical)
}

fn bridge_config_path(project_directory: &Path) -> PathBuf {
    // 每个 DaVinci Cfg 目录只读取自己的 LGK-Vector 配置。
    project_directory.join("lgk-vector.json")
}

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("path not found: {}", path.display()))?;
    let canonical = normalize_canonical_path(canonical);
    if !canonical.is_dir() {
        bail!("path is not a directory: {}", canonical.display());
    }
    Ok(canonical)
}

#[cfg(windows)]
fn normalize_canonical_path(path: PathBuf) -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    const EXTENDED_PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const EXTENDED_UNC_PREFIX: &[u16] = &[
        b'\\' as u16,
        b'\\' as u16,
        b'?' as u16,
        b'\\' as u16,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        b'\\' as u16,
    ];

    let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if starts_with_ascii_case_insensitive(&wide, EXTENDED_UNC_PREFIX) {
        let mut ordinary_unc = vec![b'\\' as u16, b'\\' as u16];
        ordinary_unc.extend_from_slice(&wide[EXTENDED_UNC_PREFIX.len()..]);
        return PathBuf::from(OsString::from_wide(&ordinary_unc));
    }
    if wide.starts_with(EXTENDED_PREFIX) {
        return PathBuf::from(OsString::from_wide(&wide[EXTENDED_PREFIX.len()..]));
    }
    path
}

#[cfg(windows)]
fn starts_with_ascii_case_insensitive(value: &[u16], prefix: &[u16]) -> bool {
    value.len() >= prefix.len()
        && value
            .iter()
            .zip(prefix)
            .all(|(left, right)| ascii_upper(*left) == ascii_upper(*right))
}

#[cfg(windows)]
fn ascii_upper(value: u16) -> u16 {
    if (b'a' as u16..=b'z' as u16).contains(&value) {
        value - (b'a' - b'A') as u16
    } else {
        value
    }
}

#[cfg(not(windows))]
fn normalize_canonical_path(path: PathBuf) -> PathBuf {
    path
}
