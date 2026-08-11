use std::io::Read;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    // 这个 EXE 是一次性 CLI：支持内联 JSON、stdin 和请求文件三种输入。
    let mut args = std::env::args_os();
    let executable = args.next().unwrap_or_else(|| "lgk-vector".into());
    let Some(raw) = args.next() else {
        anyhow::bail!(
            "Usage: {} --version | --doctor --request-file <path> | --request-file <path> | --stdin | '{{\"func\":\"find_module\",\"module\":\"Com\"}}'",
            PathBuf::from(executable).display()
        );
    };
    let raw = raw
        .into_string()
        .map_err(|_| anyhow::anyhow!("request argument is not valid Unicode"))?;
    if raw == "--version" || raw == "-V" {
        if args.next().is_some() {
            anyhow::bail!("--version does not accept another argument");
        }
        println!(
            "lgk-vector {} protocol={} build={}",
            env!("CARGO_PKG_VERSION"),
            lgk_vector::app::HOST_PROTOCOL_VERSION,
            lgk_vector::app::BUILD_ID
        );
        return Ok(());
    }
    if raw == "--start-host" {
        // 只准备后台服务，不执行具体 ECUC 请求。
        if args.next().is_some() {
            anyhow::bail!("--start-host does not accept another argument");
        }
        lgk_vector::app::cli::ensure_host()?;
        println!("resident host ready");
        return Ok(());
    }
    if raw == "--doctor" {
        let option = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("--doctor requires --request-file <path>"))?;
        if option != "--request-file" {
            anyhow::bail!("--doctor requires --request-file <path>");
        }
        let path = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("--doctor requires --request-file <path>"))?;
        if args.next().is_some() {
            anyhow::bail!("--doctor accepts exactly one request file");
        }
        let request = without_utf8_bom(std::fs::read_to_string(PathBuf::from(path))?);
        let project = std::env::current_dir()?;
        println!("{}", lgk_vector::app::cli::doctor(&project, &request)?);
        return Ok(());
    }
    let raw = match raw.as_str() {
        "--stdin" => {
            if args.next().is_some() {
                anyhow::bail!("--stdin does not accept another argument");
            }
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            input
        }
        "--request-file" => {
            let path = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("--request-file requires a path"))?;
            if args.next().is_some() {
                anyhow::bail!("--request-file accepts exactly one path");
            }
            std::fs::read_to_string(PathBuf::from(path))?
        }
        _ => {
            if args.next().is_some() {
                anyhow::bail!("expected exactly one JSON request argument");
            }
            raw
        }
    };
    let raw = without_utf8_bom(raw);
    // 当前工作目录就是 Cfg；SessionConfig 会从这里读取 lgk-vector.json。
    let project = std::env::current_dir()?;
    let output = lgk_vector::app::cli::run(&project, &raw)?;
    println!("{output}");
    Ok(())
}

fn without_utf8_bom(raw: String) -> String {
    raw.strip_prefix('\u{feff}').unwrap_or(&raw).to_string()
}
