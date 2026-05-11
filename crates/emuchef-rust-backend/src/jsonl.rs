use crate::envelope;
use crate::errors::ApiError;
use crate::request;
use crate::session::DocumentSessionManager;

/// Process UTF-8 JSON Lines input and return JSON Lines output.
///
/// Each input line produces exactly one response line. Request-level protocol
/// errors are encoded as `ok: false` envelopes and do not stop processing.
pub fn process_jsonl(input: &str) -> String {
    let mut output = String::new();
    let mut sessions = DocumentSessionManager::default();
    for line in input.lines() {
        let response = match serde_json::from_str(line) {
            Ok(request) => request::handle_sidecar_value(request, &mut sessions),
            Err(_) => envelope::with_id(
                envelope::failure(ApiError::invalid_request("Malformed JSON line")),
                None,
            ),
        };
        output.push_str(
            &serde_json::to_string(&response).expect("serializing JSON response should not fail"),
        );
        output.push('\n');
    }
    output
}
