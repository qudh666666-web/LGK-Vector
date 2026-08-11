use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::app::{HostRequest, HostResponse, HOST_PROTOCOL_VERSION};
use crate::daemon::commands::CommandDispatcher;
use crate::project::SessionConfig;

const HOST_READY: u8 = 0;
const HOST_BUSY: u8 = 1;
const HOST_STOPPING: u8 = 2;

// Resident Host 只服务本机的一个 DaVinci 工程会话。
// 这样连续请求可复用 DaVinci，避免每次查询或生成都冷启动。
pub fn run(port: u16, token_path: &Path) -> Result<()> {
    let token = fs::read_to_string(token_path)
        .with_context(|| format!("read token file: {}", token_path.display()))?;
    let token = token.trim().to_string();
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("resident token must contain exactly 64 hexadecimal characters");
    }
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener =
        TcpListener::bind(address).with_context(|| format!("bind resident server: {address}"))?;
    let stopping = Arc::new(AtomicBool::new(false));
    let activity = Arc::new(AtomicU8::new(HOST_READY));
    let probe_port = port
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("resident probe port overflow"))?;
    let probe_handle = start_probe_listener(
        probe_port,
        token.clone(),
        Arc::clone(&stopping),
        Arc::clone(&activity),
    )?;
    // 第一个有效请求绑定工程；会话期间拒绝切换工程，防止串项目。
    let mut active_config: Option<SessionConfig> = None;
    // Dispatcher 持有惰性创建的 DaVinciClient。
    let mut dispatcher = Some(CommandDispatcher::new());

    // 每条 TCP 连接只承载一个 JSON 请求和一个 JSON 响应。
    for incoming in listener.incoming() {
        let mut stream = match incoming {
            Ok(stream) => stream,
            Err(_) => continue,
        };
        if stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .is_err()
        {
            continue;
        }
        let request = match read_request(&mut stream) {
            Ok(request) => request,
            Err(error) => {
                let _ = write_response(
                    &mut stream,
                    HostResponse {
                        protocol_version: HOST_PROTOCOL_VERSION,
                        exit_code: 2,
                        stdout: String::new(),
                        stderr: format!("invalid resident request: {error}"),
                    },
                );
                continue;
            }
        };
        if request.protocol_version != HOST_PROTOCOL_VERSION {
            let _ = write_response(
                &mut stream,
                HostResponse {
                    protocol_version: HOST_PROTOCOL_VERSION,
                    exit_code: 5,
                    stdout: String::new(),
                    stderr: format!(
                        "resident host protocol mismatch: request={}, host={}",
                        request.protocol_version, HOST_PROTOCOL_VERSION
                    ),
                },
            );
            continue;
        }
        // 端口只绑定 127.0.0.1，Token 再限制为包装器创建的本地会话。
        if request.launch_key != token {
            let _ = write_response(
                &mut stream,
                HostResponse {
                    protocol_version: HOST_PROTOCOL_VERSION,
                    exit_code: 3,
                    stdout: String::new(),
                    stderr: "invalid launch key".to_string(),
                },
            );
            continue;
        }

        if request.probe {
            let status = activity_name(activity.load(Ordering::Acquire));
            let _ = write_response(
                &mut stream,
                HostResponse {
                    protocol_version: HOST_PROTOCOL_VERSION,
                    exit_code: 0,
                    stdout: serde_json::json!({
                        "status": status,
                        "version": env!("CARGO_PKG_VERSION"),
                        "build_id": crate::app::BUILD_ID
                    })
                    .to_string(),
                    stderr: String::new(),
                },
            );
            continue;
        }

        // shutdown 不执行业务请求。协议和 Token 已确认后直接关闭，
        // 即使工程 JSON 在会话期间被修改或损坏，也不能把旧 Host 困住。
        if request.shutdown {
            activity.store(HOST_STOPPING, Ordering::Release);
            let result = dispatcher.take().expect("dispatcher available").shutdown();
            let response = match result {
                Ok(()) => HostResponse {
                    protocol_version: HOST_PROTOCOL_VERSION,
                    exit_code: 0,
                    stdout: serde_json::json!({"status": "shutdown"}).to_string(),
                    stderr: String::new(),
                },
                Err(error) => HostResponse {
                    protocol_version: HOST_PROTOCOL_VERSION,
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: error.to_string(),
                },
            };
            let _ = write_response(&mut stream, response);
            stopping.store(true, Ordering::Release);
            break;
        }

        let Some(request_config) = request.cfg.as_ref() else {
            let _ = write_response(
                &mut stream,
                HostResponse {
                    protocol_version: HOST_PROTOCOL_VERSION,
                    exit_code: 2,
                    stdout: String::new(),
                    stderr: "project configuration is required for a normal request".to_string(),
                },
            );
            continue;
        };
        if let Some(active) = &active_config {
            if active != request_config {
                let _ = write_response(
                    &mut stream,
                    HostResponse {
                        protocol_version: HOST_PROTOCOL_VERSION,
                        exit_code: 4,
                        stdout: String::new(),
                        stderr: format!(
                            "resident host is already bound to another project: active={}, requested={}",
                            active.project_path.display(),
                            request_config.project_path.display()
                        ),
                    },
                );
                continue;
            }
        } else {
            active_config = Some(request_config.clone());
        }

        // 普通请求交给命令分发器：本地查询不会启动 DaVinci，
        // 只有生成/校验类请求才会按需创建 DaVinciClient。
        activity.store(HOST_BUSY, Ordering::Release);
        let response = match dispatcher
            .as_mut()
            .expect("dispatcher available")
            .dispatch_batch(request_config, &request.raw_text)
        {
            Ok(value) => HostResponse {
                protocol_version: HOST_PROTOCOL_VERSION,
                exit_code: 0,
                stdout: serde_json::to_string_pretty(&value)?,
                stderr: String::new(),
            },
            Err(error) => HostResponse {
                protocol_version: HOST_PROTOCOL_VERSION,
                exit_code: 1,
                stdout: String::new(),
                stderr: error.to_string(),
            },
        };
        let _ = write_response(&mut stream, response);
        activity.store(HOST_READY, Ordering::Release);
    }
    stopping.store(true, Ordering::Release);
    let _ = probe_handle.join();
    Ok(())
}

fn start_probe_listener(
    port: u16,
    token: String,
    stopping: Arc<AtomicBool>,
    activity: Arc<AtomicU8>,
) -> Result<thread::JoinHandle<()>> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener =
        TcpListener::bind(address).with_context(|| format!("bind resident probe: {address}"))?;
    listener.set_nonblocking(true)?;
    Ok(thread::spawn(move || {
        while !stopping.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let response = match read_request(&mut stream) {
                        Ok(request)
                            if request.protocol_version == HOST_PROTOCOL_VERSION
                                && request.launch_key == token
                                && request.probe =>
                        {
                            let status = activity_name(activity.load(Ordering::Acquire));
                            HostResponse {
                                protocol_version: HOST_PROTOCOL_VERSION,
                                exit_code: 0,
                                stdout: serde_json::json!({
                                    "status": status,
                                    "version": env!("CARGO_PKG_VERSION"),
                                    "build_id": crate::app::BUILD_ID
                                })
                                .to_string(),
                                stderr: String::new(),
                            }
                        }
                        Ok(_) => HostResponse {
                            protocol_version: HOST_PROTOCOL_VERSION,
                            exit_code: 2,
                            stdout: String::new(),
                            stderr: "invalid resident probe identity".to_string(),
                        },
                        Err(error) => HostResponse {
                            protocol_version: HOST_PROTOCOL_VERSION,
                            exit_code: 2,
                            stdout: String::new(),
                            stderr: format!("invalid resident probe: {error}"),
                        },
                    };
                    // A timed-out caller may already have closed its socket. A
                    // failed health response must never stop the business Host.
                    let _ = write_response(&mut stream, response);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
    }))
}

fn activity_name(value: u8) -> &'static str {
    match value {
        HOST_BUSY => "busy",
        HOST_STOPPING => "stopping",
        _ => "ready",
    }
}

fn read_request(stream: &mut TcpStream) -> Result<HostRequest> {
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    serde_json::from_str(line.trim()).context("parse resident request")
}

fn write_response(stream: &mut TcpStream, response: HostResponse) -> Result<()> {
    serde_json::to_writer(&mut *stream, &response)?;
    writeln!(stream)?;
    stream.flush()?;
    Ok(())
}
