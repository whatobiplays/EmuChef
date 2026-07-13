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
    "emitUserConfigurationYamlFromPath",
    "validateUserConfigurationPath",
    "openUserConfiguration",
    "createUserConfiguration",
    "getUserConfigurationDocument",
    "saveUserConfiguration",
    "saveUserConfigurationAs",
    "setUserConfigurationBinding",
    "removeUserConfigurationBinding",
    "setUserConfigurationSelectedRecipes",
    "setUserConfigurationDevicePlan",
    "validateUserConfiguration",
    "emitUserConfigurationYaml",
    "setUserConfigurationAuthoredRoot",
    "closeUserConfiguration",
    "describeConfiguration",
    "openRecipe",
    "createRecipeFromTemplate",
    "getDocument",
    "saveRecipe",
    "saveRecipeAs",
    "closeDocument",
    "applyRecipeCommand",
    "undo",
    "redo",
    "emitYaml",
    "validate",
    "getRefIndex",
    "setDocumentAuthoredRoot",
    "ping",
];

/// Return backend compatibility metadata for the `hello` request.
pub fn hello_result() -> Value {
    json!({
        "protocolVersion": SUPPORTED_PROTOCOL_VERSION,
        "capabilities": CAPABILITIES,
    })
}

/// Return lightweight sidecar health metadata for the `ping` request.
pub fn ping_result() -> Value {
    json!({
        "healthy": true,
    })
}
