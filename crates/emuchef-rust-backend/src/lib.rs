//! Standalone protocol skeleton for the experimental Rust editor backend.
//!
//! The crate intentionally implements only transport-level behavior for Phase
//! 6C and 6D: request validation, response envelopes, one-shot invocation,
//! JSONL sidecar invocation, the `hello` handshake, and static StepSpec DTO
//! parity for `listStepSpecs`.

pub mod envelope;
pub mod errors;
pub mod jsonl;
pub mod one_shot;
pub mod protocol;
pub mod request;
pub mod step_specs;

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

    one_shot::run(args)
}
