//! EmuChef's canonical Rust CLI, executor, planner, validation, and sidecar runtime.
//!
//! The crate exposes the product `emuchef` executable and the one-shot and JSONL
//! protocol surfaces used by the Tauri configuration editor and the additive
//! Phase 0 contract for a future end-user application.

/// Production Android permission catalog and pure classifier.
pub(crate) mod android_permissions;
/// Native authoring-time APK inspection and review DTOs.
pub(crate) mod apk_authoring_inspection;
/// Production APK manifest inspection boundary for hostile APK input.
pub(crate) mod apk_manifest;
mod artifact_resolver;
mod artifact_store;
mod artifact_transport;
pub mod authored_models;
pub mod catalog;
pub mod catalog_source;
mod cli;
pub mod commands;
pub(crate) mod device_probe;
pub(crate) mod device_profile_match;
pub mod document;
pub mod dto;
mod end_user_runtime;
pub mod envelope;
pub mod errors;
pub mod execution_session;
mod executor;
#[cfg(test)]
mod executor_real_adb_tests;
#[cfg(test)]
mod executor_tests;
pub(crate) mod generation;
pub mod jsonl;
pub mod model;
pub mod one_shot;
mod owned_process;
mod plan_digest;
mod planner;
pub(crate) mod planner_device_plan;
mod planner_runtime;
#[cfg(test)]
mod planner_tests;
mod product_catalog;
pub mod protocol;
mod raw_request;
#[cfg(test)]
mod recipe_qualification_bios_tests;
#[cfg(test)]
mod recipe_qualification_retroarch_tests;
pub mod ref_index;
mod remote_release_resolver;
pub mod request;
mod review_projection;
pub mod runtime_configuration;
pub(crate) mod runtime_refs;
pub mod session;
pub mod step_specs;
pub mod user_configuration;
pub mod user_configuration_document;
pub mod validation;
pub mod yaml;

#[derive(Debug, PartialEq, Eq)]
/// Captured process output used by the binary and process-level tests.
pub struct ProcessOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Route command-line arguments to either one-shot mode or JSONL sidecar mode.
///
/// `--sidecar` is a strict mode selector. Combining it with a request argument
/// is treated as process-level misuse, so the function returns a non-zero exit
/// code and does not emit a protocol envelope on stdout.
pub fn run_with_args_and_input(args: &[String], input: &str) -> ProcessOutput {
    if args.first().is_some_and(|arg| arg == "--sidecar") {
        if args.len() != 1 {
            return ProcessOutput {
                exit_code: 2,
                stdout: String::new(),
                stderr: "usage: emuchef --sidecar\n".to_string(),
            };
        }
        return ProcessOutput {
            exit_code: 0,
            stdout: jsonl::process_jsonl(input),
            stderr: String::new(),
        };
    }

    if args.first().is_some_and(|arg| cli::is_cli_command(arg)) {
        return cli::run(args);
    }

    one_shot::run(args)
}
