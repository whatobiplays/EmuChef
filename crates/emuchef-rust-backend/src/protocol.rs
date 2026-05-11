use serde_json::{json, Value};

/// Protocol version understood by this skeleton.
pub const SUPPORTED_PROTOCOL_VERSION: u64 = 1;

/// Editor capabilities implemented by this experimental backend.
///
/// `hello` is the handshake request and is intentionally not listed as a
/// capability. Future phases should add capability names only after the
/// corresponding editor operation is implemented.
pub const CAPABILITIES: &[&str] = &[
    "listStepSpecs",
    "emitRecipeYamlFromPath",
    "validateRecipePath",
    "openRecipe",
    "getDocument",
    "saveRecipe",
    "closeDocument",
];

/// Return backend compatibility metadata for the `hello` request.
pub fn hello_result() -> Value {
    json!({
        "protocolVersion": SUPPORTED_PROTOCOL_VERSION,
        "capabilities": CAPABILITIES,
    })
}
