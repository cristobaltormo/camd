use std::process::ExitCode;

const USAGE: &str =
    "usage: camd [-h] [--version] [serve | ctl {enroll,password,revoke-all,devices}]";

fn serve() -> ExitCode {
    let configs = match camd::config::load_all(None) {
        Ok(configs) => configs,
        Err(e) => {
            camd::error!("camd", "{}", e);
            return ExitCode::from(1);
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(camd::WORKER_THREADS)
        .max_blocking_threads(camd::MAX_BLOCKING_THREADS)
        .thread_name("camd-worker")
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(rt) => rt,
        Err(e) => {
            camd::error!("camd", "cannot start the async runtime: {}", e);
            return ExitCode::from(1);
        }
    };
    match runtime.block_on(camd::app::run(configs)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            camd::error!("camd", "{}", e);
            ExitCode::from(1)
        }
    }
}

fn ctl(args: &[String]) -> ExitCode {
    let code = camd::ctl::run(args, &mut camd::ctl::Terminal);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn main() -> ExitCode {
    let mut argv = std::env::args();
    let program = argv.next().unwrap_or_default();
    let args: Vec<String> = argv.collect();
    let invoked_as_ctl = std::path::Path::new(&program)
        .file_name()
        .is_some_and(|name| name == "camd-ctl");
    if invoked_as_ctl {
        return ctl(&args);
    }
    match args.first().map(String::as_str) {
        None | Some("serve") if args.len() <= 1 => serve(),
        Some("ctl") => ctl(&args[1..]),
        Some("--version") | Some("-V") if args.len() == 1 => {
            println!("camd {}", camd::VERSION);
            ExitCode::SUCCESS
        }
        Some("-h") | Some("--help") => {
            println!("{USAGE}");
            println!();
            println!("Self-hosted ESP32-CAM server");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            eprintln!("camd: error: unrecognized arguments: {}", args.join(" "));
            ExitCode::from(2)
        }
    }
}
