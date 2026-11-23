use crate::envelope;
use crate::execution_session::SidecarRuntimeConfig;
use crate::request;
use crate::session::DocumentSessionManager;

use std::io::{self, BufRead, Write};

/// Process UTF-8 JSON Lines input and return JSON Lines output.
///
/// Each input line produces exactly one response line. Request-level protocol
/// errors are encoded as `ok: false` envelopes and do not stop processing.
pub fn process_jsonl(input: &str) -> String {
    process_jsonl_with_config(input, SidecarRuntimeConfig::default())
}

/// Process JSON Lines with filesystem policy fixed for this sidecar process.
pub fn process_jsonl_with_config(input: &str, config: SidecarRuntimeConfig) -> String {
    process_jsonl_with_restriction(input, config, None)
}

/// Process JSON Lines with startup-bound restricted execution identity.
pub fn process_jsonl_with_restriction(
    input: &str,
    config: SidecarRuntimeConfig,
    restricted_build_identity: Option<String>,
) -> String {
    process_jsonl_with_startup_policy(input, config, restricted_build_identity, None)
}

/// Process JSON Lines with startup-bound restriction and native generation.
pub fn process_jsonl_with_startup_policy(
    input: &str,
    config: SidecarRuntimeConfig,
    restricted_build_identity: Option<String>,
    runtime_generation: Option<u64>,
) -> String {
    let mut output = String::new();
    let mut sessions = DocumentSessionManager::with_runtime_startup_policy(
        config,
        restricted_build_identity,
        runtime_generation,
    );
    for line in input.lines() {
        let response = match crate::raw_request::parse(line) {
            Ok(request) => request::handle_sidecar_value(request, &mut sessions),
            Err(error) => envelope::with_id(
                envelope::failure(error.api_error("Malformed JSON line")),
                error.request_id(),
            ),
        };
        output.push_str(
            &serde_json::to_string(&response).expect("serializing JSON response should not fail"),
        );
        output.push('\n');
    }
    output
}

/// Run the sidecar JSON Lines protocol over interactive streams.
///
/// The Tauri client writes one request line and then waits for one response line,
/// so the real sidecar process must flush after every response instead of
/// buffering until stdin reaches EOF.
pub fn run_jsonl_sidecar<R, W>(mut reader: R, mut writer: W) -> io::Result<()>
where
    R: BufRead,
    W: Write,
{
    run_jsonl_sidecar_with_config(&mut reader, &mut writer, SidecarRuntimeConfig::default())
}

/// Run an interactive sidecar with roots and ADB policy fixed at startup.
pub fn run_jsonl_sidecar_with_config<R, W>(
    mut reader: R,
    mut writer: W,
    config: SidecarRuntimeConfig,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
{
    run_jsonl_sidecar_with_restriction(&mut reader, &mut writer, config, None)
}

/// Run an interactive sidecar with an optional startup-bound execution restriction.
pub fn run_jsonl_sidecar_with_restriction<R, W>(
    mut reader: R,
    mut writer: W,
    config: SidecarRuntimeConfig,
    restricted_build_identity: Option<String>,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
{
    run_jsonl_sidecar_with_startup_policy(
        &mut reader,
        &mut writer,
        config,
        restricted_build_identity,
        None,
    )
}

/// Run an interactive sidecar bound to trusted native startup identity.
pub fn run_jsonl_sidecar_with_startup_policy<R, W>(
    reader: R,
    writer: W,
    config: SidecarRuntimeConfig,
    restricted_build_identity: Option<String>,
    runtime_generation: Option<u64>,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
{
    run_jsonl_sidecar_inner(
        reader,
        writer,
        config,
        restricted_build_identity,
        runtime_generation,
        None,
    )
}

pub fn run_jsonl_sidecar_with_trusted_control<R, W>(
    reader: R,
    writer: W,
    config: SidecarRuntimeConfig,
    restricted_build_identity: Option<String>,
    runtime_generation: u64,
    key: [u8; 32],
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
{
    run_jsonl_sidecar_inner(
        reader,
        writer,
        config,
        restricted_build_identity,
        Some(runtime_generation),
        Some(crate::trusted_control::TrustedControlVerifier::new(
            key,
            runtime_generation,
        )),
    )
}

fn run_jsonl_sidecar_inner<R, W>(
    mut reader: R,
    mut writer: W,
    config: SidecarRuntimeConfig,
    restricted_build_identity: Option<String>,
    runtime_generation: Option<u64>,
    mut trusted_control: Option<crate::trusted_control::TrustedControlVerifier>,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
{
    let mut sessions = DocumentSessionManager::with_runtime_startup_policy(
        config,
        restricted_build_identity,
        runtime_generation,
    );
    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }

        let response = match crate::raw_request::parse(line.trim_end_matches(['\r', '\n'])) {
            Ok(mut request) => {
                let control = match trusted_control.as_mut() {
                    Some(verifier) => verifier.verify_request(&mut request),
                    None => crate::trusted_control::reject_unconfigured_control(&request),
                };
                match control {
                    Ok(control) => {
                        request::handle_sidecar_value_with_control(request, &mut sessions, control)
                    }
                    Err(error) => {
                        let request_id = request
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string);
                        envelope::with_id(envelope::failure(error), request_id)
                    }
                }
            }
            Err(error) => envelope::with_id(
                envelope::failure(error.api_error("Malformed JSON line")),
                error.request_id(),
            ),
        };
        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}
