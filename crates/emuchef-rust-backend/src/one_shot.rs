use crate::envelope;
use crate::errors::ApiError;
use crate::request;
use crate::ProcessOutput;

/// Handle non-sidecar CLI invocation.
///
/// One-shot mode treats missing, extra, malformed, or semantically invalid
/// request arguments as API-level failures, so those cases still return exit
/// code 0 with exactly one JSON envelope on stdout.
pub fn run(args: &[String]) -> ProcessOutput {
    let response = if args.len() != 1 {
        envelope::failure(ApiError::invalid_request(
            "Expected exactly one JSON request argument.",
        ))
    } else {
        match crate::raw_request::parse(&args[0]) {
            Ok(request) => request::handle_one_shot_value(request),
            Err(error) => envelope::failure(error.api_error("Invalid JSON request.")),
        }
    };

    ProcessOutput {
        exit_code: 0,
        stdout: format!(
            "{}\n",
            serde_json::to_string(&response).expect("serializing JSON response should not fail")
        ),
        stderr: String::new(),
    }
}
