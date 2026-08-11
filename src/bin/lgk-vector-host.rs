use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    // 这个 EXE 只启动 TCP Host；正常情况下由 PowerShell 包装器或 CLI 拉起。
    let mut port = None;
    let mut token_file = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--version" | "-V" => {
                if args.next().is_some() {
                    anyhow::bail!("--version does not accept another argument");
                }
                println!(
                    "lgk-vector-host {} protocol={} build={}",
                    env!("CARGO_PKG_VERSION"),
                    lgk_vector::app::HOST_PROTOCOL_VERSION,
                    lgk_vector::app::BUILD_ID
                );
                return Ok(());
            }
            "--port" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--port requires a value"))?;
                port = Some(value.to_string_lossy().parse::<u16>()?);
            }
            "--token-file" => {
                token_file =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--token-file requires a value")
                    })?));
            }
            other => anyhow::bail!("unsupported argument: {other}"),
        }
    }
    let port = port.unwrap_or(lgk_vector::app::DEFAULT_HOST_PORT);
    let token_file = token_file.unwrap_or_else(lgk_vector::app::token_file);
    // Host 的业务循环、Token 校验和正常关闭都在 app::host 中实现。
    lgk_vector::app::host::run(port, &token_file)
}
