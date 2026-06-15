//! Standalone protocol skeleton for the experimental Rust editor backend.
//!
//! The crate intentionally implements only the migration surface from Phases 6C
//! through 6S: request validation, response envelopes, one-shot invocation,
//! JSONL sidecar invocation, the `hello` handshake, Rust-owned StepSpec DTOs,
//! authored recipe YAML load/emit/validation fixtures, sidecar document
//! sessions, editor command parity, undo/redo, fixture-scoped RefIndex
//! generation, authoredRoot catalog-context validation, internal planner
//! fixtures, and an internal executor with temp-dir-confined filesystem/artifact
//! fixture behavior plus selected fake-device/DryRunAdb parity, real-ADB adapter
//! foundations, and a minimal crate-local CLI parity skeleton. The Rust executor
//! is not exposed through protocol, Tauri, backend selection, or production
//! packaging APIs.

pub mod catalog;
mod cli;
pub mod commands;
#[allow(dead_code)]
pub(crate) mod device_probe;
#[allow(dead_code)]
pub(crate) mod device_profile_match;
pub mod document;
pub mod dto;
pub mod envelope;
pub mod errors;
#[allow(dead_code)]
mod executor;
#[cfg(test)]
mod executor_real_adb_tests;
#[cfg(test)]
mod executor_tests;
pub mod jsonl;
pub mod model;
pub mod one_shot;
pub mod plan_shadow;
#[allow(dead_code)]
mod planner;
#[allow(dead_code)]
pub(crate) mod planner_device_plan;
#[cfg(test)]
mod planner_tests;
pub mod protocol;
pub mod ref_index;
pub mod request;
pub mod session;
pub mod step_specs;
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
                stderr: "usage: emuchef-rust-backend --sidecar\n".to_string(),
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
