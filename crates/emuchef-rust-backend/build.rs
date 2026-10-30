use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let source = PathBuf::from("src/executor/copy_commit_helper.c");
    println!("cargo:rerun-if-changed={}", source.display());
    for name in [
        "ANDROID_NDK_HOME",
        "ANDROID_NDK_ROOT",
        "ANDROID_SDK_ROOT",
        "ANDROID_HOME",
        "CC",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    if env::var_os("CARGO_FEATURE_ANDROID_COPY_COMMIT_HELPER").is_none() {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let ndk = find_ndk()
        .unwrap_or_else(|| panic!("android-copy-commit-helper requires an installed Android NDK"));
    let host_tag = match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("windows", "x86_64") => "windows-x86_64",
        target => panic!("Android NDK host toolchain is unsupported for {target:?}"),
    };
    let prebuilt = ndk.join("toolchains/llvm/prebuilt");
    let expected_toolchain = prebuilt.join(host_tag);
    let (toolchain, has_native_ndk_tools) = if expected_toolchain.is_dir() {
        (expected_toolchain, true)
    } else if env::consts::OS == "macos"
        && env::consts::ARCH == "aarch64"
        && prebuilt.join("darwin-x86_64").is_dir()
    {
        // Apple Clang and Rust's native rust-lld can use the NDK sysroot and
        // runtime archives without executing the NDK's x86_64 host binaries.
        (prebuilt.join("darwin-x86_64"), false)
    } else {
        panic!("Android NDK host tools are missing for {host_tag}");
    };
    let tool_bin = toolchain.join("bin");
    let aarch64_output = out_dir.join("copy-commit-helper-aarch64");
    let x86_64_output = out_dir.join("copy-commit-helper-x86_64");
    if has_native_ndk_tools
        && tool_bin.join("aarch64-linux-android24-clang").is_file()
        && tool_bin.join("x86_64-linux-android24-clang").is_file()
    {
        compile_android_helper(
            &tool_bin.join("aarch64-linux-android24-clang"),
            &source,
            &aarch64_output,
        );
        compile_android_helper(
            &tool_bin.join("x86_64-linux-android24-clang"),
            &source,
            &x86_64_output,
        );
    } else {
        compile_android_helper_with_host_tools(
            &toolchain,
            &source,
            &aarch64_output,
            "aarch64-linux-android24",
            "aarch64-linux-android",
            "aarch64",
        );
        compile_android_helper_with_host_tools(
            &toolchain,
            &source,
            &x86_64_output,
            "x86_64-linux-android24",
            "x86_64-linux-android",
            "x86_64",
        );
    }

    let host_compiler = env::var_os("CC").unwrap_or_else(|| "cc".into());
    let host_output = out_dir.join("copy-commit-helper-host");
    run_compiler(
        Command::new(host_compiler)
            .arg("-std=c11")
            .arg("-O2")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg(&source)
            .arg("-o")
            .arg(&host_output),
        "host contract helper",
    );

    println!(
        "cargo:rustc-env=EMU_CHEF_COPY_COMMIT_HELPER_AARCH64={}",
        out_dir.join("copy-commit-helper-aarch64").display()
    );
    println!(
        "cargo:rustc-env=EMU_CHEF_COPY_COMMIT_HELPER_X86_64={}",
        out_dir.join("copy-commit-helper-x86_64").display()
    );
    println!(
        "cargo:rustc-env=EMU_CHEF_COPY_COMMIT_HELPER_HOST={}",
        host_output.display()
    );
}

fn find_ndk() -> Option<PathBuf> {
    for name in ["ANDROID_NDK_HOME", "ANDROID_NDK_ROOT"] {
        if let Some(path) = env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
        {
            return Some(path);
        }
    }

    let mut sdk_roots = Vec::new();
    for name in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
        if let Some(path) = env::var_os(name).map(PathBuf::from) {
            sdk_roots.push(path);
        }
    }
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        sdk_roots.push(home.join("Library/Android/sdk"));
        sdk_roots.push(home.join("Android/Sdk"));
    }

    for sdk in sdk_roots {
        let ndk_root = sdk.join("ndk");
        let Ok(entries) = fs::read_dir(ndk_root) else {
            continue;
        };
        let mut versions = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        versions.sort();
        if let Some(latest) = versions.pop() {
            return Some(latest);
        }
    }
    None
}

fn compile_android_helper(compiler: &Path, source: &Path, output: &Path) {
    if !compiler.is_file() {
        panic!(
            "Android NDK helper compiler is missing: {}",
            compiler.display()
        );
    }
    run_compiler(
        Command::new(compiler)
            .arg("-std=c11")
            .arg("-O2")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg(source)
            .arg("-o")
            .arg(output),
        "Android selected-root helper",
    );
}

fn compile_android_helper_with_host_tools(
    ndk_toolchain: &Path,
    source: &Path,
    output: &Path,
    target: &str,
    sysroot_triple: &str,
    architecture: &str,
) {
    let clang = env::var_os("CC").unwrap_or_else(|| "clang".into());
    let sysroot = ndk_toolchain.join("sysroot");
    let resource_root = ndk_toolchain.join("lib/clang");
    let resource_version = fs::read_dir(&resource_root)
        .unwrap_or_else(|error| {
            panic!(
                "could not inspect Android NDK Clang resources at {}: {error}",
                resource_root.display()
            )
        })
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .max()
        .unwrap_or_else(|| {
            panic!(
                "Android NDK Clang resources are missing under {}",
                resource_root.display()
            )
        });
    let host = env::var("HOST").expect("Cargo sets HOST for build scripts");
    let rust_lld = find_rust_lld(&host).unwrap_or_else(|| {
        panic!(
            "host Clang fallback requires native rust-lld for Cargo host {host}; checked Cargo RUSTC and the active rustup toolchain"
        )
    });
    let lld_alias = output.with_file_name(format!(
        "{}-ld.lld",
        output.file_name().unwrap().to_string_lossy()
    ));
    if lld_alias.exists() {
        fs::remove_file(&lld_alias).unwrap_or_else(|error| {
            panic!(
                "could not replace linker alias {}: {error}",
                lld_alias.display()
            )
        });
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&rust_lld, &lld_alias).unwrap_or_else(|error| {
        panic!(
            "could not create Rust LLD alias {}: {error}",
            lld_alias.display()
        )
    });
    #[cfg(not(unix))]
    panic!("host Clang Android fallback requires a Unix linker alias");

    let sysroot_library = sysroot.join("usr/lib").join(sysroot_triple).join("24");
    let compiler_runtime = resource_version.join("lib/linux").join(architecture);
    run_compiler(
        Command::new(clang)
            .arg(format!("--target={target}"))
            .arg(format!("--sysroot={}", sysroot.display()))
            .arg(format!("-resource-dir={}", resource_version.display()))
            .arg(format!("--ld-path={}", lld_alias.display()))
            .arg("-rtlib=compiler-rt")
            .arg("-unwindlib=none")
            .arg(format!("-L{}", compiler_runtime.display()))
            .arg(format!("-L{}", sysroot_library.display()))
            .arg("-std=c11")
            .arg("-O2")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg(source)
            .arg("-o")
            .arg(output),
        "Android selected-root helper through host Clang and Rust LLD",
    );
}

fn find_rust_lld(host: &str) -> Option<PathBuf> {
    let mut rustc_candidates = Vec::new();
    if let Some(rustc) = env::var_os("RUSTC") {
        rustc_candidates.push(rustc);
    }
    if let Ok(output) = Command::new("rustup").args(["which", "rustc"]).output() {
        if output.status.success() {
            rustc_candidates.push(
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_string()
                    .into(),
            );
        }
    }

    for rustc in rustc_candidates {
        let Ok(output) = Command::new(rustc).args(["--print", "sysroot"]).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        let rust_lld = sysroot.join("lib/rustlib").join(host).join("bin/rust-lld");
        if rust_lld.is_file() {
            return Some(rust_lld);
        }
    }
    None
}

fn run_compiler(command: &mut Command, description: &str) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("could not start {description} compiler: {error}"));
    if !status.success() {
        panic!("{description} compilation failed with {status}");
    }
}
