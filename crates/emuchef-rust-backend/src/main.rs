use std::io;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::os::unix::io::FromRawFd;

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
        let (config, restricted_build_identity, runtime_generation, trusted_control) =
            match sidecar_startup_policy(&args[1..]) {
                Ok(policy) => policy,
                Err(message) => {
                    eprintln!("{message}");
                    std::process::exit(2);
                }
            };
        let stdin = io::stdin();
        let stdout = io::stdout();
        let result = match trusted_control {
            Some((key, generation)) => {
                emuchef_rust_backend::jsonl::run_jsonl_sidecar_with_trusted_control(
                    stdin.lock(),
                    stdout.lock(),
                    config,
                    restricted_build_identity,
                    generation,
                    key,
                )
            }
            None => emuchef_rust_backend::jsonl::run_jsonl_sidecar_with_startup_policy(
                stdin.lock(),
                stdout.lock(),
                config,
                restricted_build_identity,
                runtime_generation,
            ),
        };
        if let Err(error) = result {
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
        Option<([u8; 32], u64)>,
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
    let mut control_fd = None;
    let mut control_generation = None;
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
        if option == "--trusted-control-fd" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "missing value for --trusted-control-fd".to_string())?;
            control_fd = Some(
                value
                    .parse::<i32>()
                    .map_err(|_| "invalid value for --trusted-control-fd".to_string())?,
            );
            index += 2;
            continue;
        }
        if option == "--control-generation" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "missing value for --control-generation".to_string())?;
            control_generation = Some(
                value
                    .parse::<u64>()
                    .map_err(|_| "invalid value for --control-generation".to_string())?,
            );
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
    let trusted_control = match (control_fd, control_generation) {
        (Some(fd), Some(generation)) => Some((read_control_key(fd)?, generation)),
        (None, None) => None,
        (None, Some(_)) => {
            return Err("control generation requires a private control descriptor".to_string())
        }
        (Some(_), None) => {
            return Err("private control key requires a control generation".to_string())
        }
    };
    Ok((
        config,
        restricted_build_identity,
        runtime_generation,
        trusted_control,
    ))
}

#[cfg(unix)]
fn read_control_key(fd: i32) -> Result<[u8; 32], String> {
    if fd < 0 {
        return Err("private control descriptor is invalid".to_string());
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut key = [0u8; 32];
    file.read_exact(&mut key)
        .map_err(|_| "private runtime session could not be initialized".to_string())?;
    let mut trailing = [0u8; 1];
    if file
        .read(&mut trailing)
        .map_err(|_| "private runtime session could not be initialized".to_string())?
        != 0
    {
        key.fill(0);
        return Err("private control bootstrap data is invalid".to_string());
    }
    Ok(key)
}

#[cfg(not(unix))]
fn read_control_key(_fd: i32) -> Result<[u8; 32], String> {
    Err("private control transport is unavailable on this platform".to_string())
}
