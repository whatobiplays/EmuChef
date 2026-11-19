use std::io;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--qualification-build-provenance"] {
        let Some(provenance) = emuchef_rust_backend::qualification_build::sidecar_provenance()
        else {
            eprintln!(
                "qualification provenance requires real-execution and android-copy-commit-helper"
            );
            std::process::exit(2);
        };
        println!("{provenance}");
        return;
    }
    if args.first().is_some_and(|argument| argument == "--sidecar") {
        let (config, restricted_build_identity, runtime_generation) =
            match sidecar_startup_policy(&args[1..]) {
                Ok(policy) => policy,
                Err(message) => {
                    eprintln!("{message}");
                    std::process::exit(2);
                }
            };
        let stdin = io::stdin();
        let stdout = io::stdout();
        if let Err(error) = emuchef_rust_backend::jsonl::run_jsonl_sidecar_with_startup_policy(
            stdin.lock(),
            stdout.lock(),
            config,
            restricted_build_identity,
            runtime_generation,
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

fn sidecar_startup_policy(
    args: &[String],
) -> Result<
    (
        emuchef_rust_backend::execution_session::SidecarRuntimeConfig,
        Option<String>,
        Option<u64>,
    ),
    String,
> {
    let restricted_build_identity =
        emuchef_rust_backend::qualification_build::restricted_execution_policy(
            args,
            cfg!(debug_assertions),
            cfg!(feature = "real-execution"),
            std::env::var("EMUCHEF_DEVICE_QUALIFICATION")
                .ok()
                .as_deref()
                == Some("1"),
            emuchef_rust_backend::qualification_build::embedded_restricted_execution_build_identity(
            ),
        )?;
    let runtime_generation =
        emuchef_rust_backend::qualification_build::restricted_execution_runtime_generation(
            args,
            restricted_build_identity.is_some(),
        )?;
    let mut config = emuchef_rust_backend::execution_session::SidecarRuntimeConfig::default();
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        if option == "--restrict-privileged-execution" {
            index += 1;
            continue;
        }
        if option == "--qualification-source-identity" {
            index += 2;
            continue;
        }
        if option == "--runtime-generation" {
            index += 2;
            continue;
        }
        if !matches!(option, "--runtime-root" | "--cache-root" | "--adb") {
            return Err(format!(
                "unknown sidecar option: {option}\nusage: emuchef --sidecar [--runtime-root PATH] [--cache-root PATH] [--adb PATH] [--restrict-privileged-execution --qualification-source-identity JSON --runtime-generation N]"
            ));
        }
        index += 1;
        let value = args.get(index).ok_or_else(|| {
            format!(
                "missing value for {option}\nusage: emuchef --sidecar [--runtime-root PATH] [--cache-root PATH] [--adb PATH] [--restrict-privileged-execution --qualification-source-identity JSON --runtime-generation N]"
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
    Ok((config, restricted_build_identity, runtime_generation))
}
