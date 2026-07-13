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
    let mut output = String::new();
    let mut sessions = DocumentSessionManager::with_runtime_config(config);
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
    let mut sessions = DocumentSessionManager::with_runtime_config(config);
    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }

        let response = match crate::raw_request::parse(line.trim_end_matches(['\r', '\n'])) {
            Ok(request) => request::handle_sidecar_value(request, &mut sessions),
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
