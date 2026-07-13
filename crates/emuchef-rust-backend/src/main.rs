use std::io;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|argument| argument == "--sidecar") {
        let config = match sidecar_runtime_config(&args[1..]) {
            Ok(config) => config,
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(2);
            }
        };
        let stdin = io::stdin();
        let stdout = io::stdout();
        if let Err(error) = emuchef_rust_backend::jsonl::run_jsonl_sidecar_with_config(
            stdin.lock(),
            stdout.lock(),
            config,
        ) {
            eprintln!("sidecar failed: {error}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    let output = emuchef_rust_backend::run_with_args_and_input(&args, "");
    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    std::process::exit(output.exit_code);
}

fn sidecar_runtime_config(
    args: &[String],
) -> Result<emuchef_rust_backend::execution_session::SidecarRuntimeConfig, String> {
    let mut config = emuchef_rust_backend::execution_session::SidecarRuntimeConfig::default();
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        if !matches!(option, "--runtime-root" | "--cache-root" | "--adb") {
            return Err(format!(
                "unknown sidecar option: {option}\nusage: emuchef --sidecar [--runtime-root PATH] [--cache-root PATH] [--adb PATH]"
            ));
        }
        index += 1;
        let value = args.get(index).ok_or_else(|| {
            format!(
                "missing value for {option}\nusage: emuchef --sidecar [--runtime-root PATH] [--cache-root PATH] [--adb PATH]"
            )
        })?;
        match option {
            "--runtime-root" => config.runtime_root = value.into(),
            "--cache-root" => config.cache_root = value.into(),
            "--adb" => config.adb_path = value.clone(),
            _ => unreachable!(),
        }
        index += 1;
    }
    Ok(config)
}
