pub mod cli;
pub mod host;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::project::SessionConfig;

// CLI 和 Host 共用的固定本机入口；DaVinci 内 Groovy 使用随机端口。
pub const DEFAULT_HOST_PORT: u16 = 32483;
pub const DEFAULT_HOST_PROBE_PORT: u16 = DEFAULT_HOST_PORT + 1;
pub const HOST_PROTOCOL_VERSION: u32 = 2;
pub const BUILD_ID: &str = match option_env!("LGK_VECTOR_BUILD_ID") {
    Some(value) => value,
    None => "dev",
};

// CLI 发给 Host 的本地 TCP 信封：业务请求、已校验配置和 Token 一起传递。
#[derive(Debug, Serialize, Deserialize)]
pub struct HostRequest {
    pub protocol_version: u32,
    pub raw_text: String,
    #[serde(default)]
    pub cfg: Option<SessionConfig>,
    pub launch_key: String,
    pub shutdown: bool,
    #[serde(default)]
    pub probe: bool,
}

// Host 总是把成功输出和失败信息分开返回，CLI 再决定是否以非零退出。
#[derive(Debug, Serialize, Deserialize)]
pub struct HostResponse {
    pub protocol_version: u32,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn token_file() -> PathBuf {
    // Token 放在 EXE 目录旁边，使包装器、CLI 和 Host 能够稳定共享它。
    let base = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(".lgk-vector").join("host.token")
}

#[cfg(test)]
mod tests {
    use super::HostResponse;

    #[test]
    fn rejects_response_from_legacy_host_without_protocol_version() {
        let legacy = r#"{"exit_code":0,"stdout":"ok","stderr":""}"#;
        assert!(serde_json::from_str::<HostResponse>(legacy).is_err());
    }
}
