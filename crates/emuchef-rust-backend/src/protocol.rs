use serde_json::{json, Value};

/// Protocol version understood by this skeleton.
pub const SUPPORTED_PROTOCOL_VERSION: u64 = 1;
pub const PHASE0_EXTENSION_ID: &str = "phase0_end_user_runtime";
pub const PHASE0_EXTENSION_VERSION: u64 = 1;

/// Editor and additive product capabilities implemented by the Rust backend.
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
    "describeCatalog",
    "listAdbDevices",
    "probeDevice",
    "qualifyConnectedDevice",
    "inspectApk",
    "generateAppRecipeDraft",
    "generateRemoteAppRecipeDraft",
    "generateDeviceProfileDraft",
    "checkGeneratedCatalogCollisions",
    "matchDevice",
    "negotiateCapabilities",
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
    "planConfiguration",
    "startExecution",
    "getExecution",
    "getExecutionEvents",
    "cancelExecution",
    "launchExecutionApp",
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
        "protocolExtensions": [{
            "id": PHASE0_EXTENSION_ID,
            "version": PHASE0_EXTENSION_VERSION,
        }],
    })
}

/// Negotiate the additive end-user runtime surface before a client relies on
/// it. Unknown required capabilities make the result incompatible; unknown
/// optional capabilities are reported but do not prevent use.
pub fn negotiate_capabilities(required: &[String], optional: &[String]) -> Value {
    let supported = |capability: &&String| CAPABILITIES.contains(&capability.as_str());
    let unsupported_required = required
        .iter()
        .filter(|capability| !CAPABILITIES.contains(&capability.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let enabled_required = required
        .iter()
        .filter(supported)
        .cloned()
        .collect::<Vec<_>>();
    let enabled_optional = optional
        .iter()
        .filter(supported)
        .cloned()
        .collect::<Vec<_>>();
    let unsupported_optional = optional
        .iter()
        .filter(|capability| !CAPABILITIES.contains(&capability.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    json!({
        "compatible": unsupported_required.is_empty(),
        "protocolVersion": SUPPORTED_PROTOCOL_VERSION,
        "extension": {
            "id": PHASE0_EXTENSION_ID,
            "version": PHASE0_EXTENSION_VERSION,
        },
        "enabledRequired": enabled_required,
        "unsupportedRequired": unsupported_required,
        "enabledOptional": enabled_optional,
        "unsupportedOptional": unsupported_optional,
    })
}

/// Return lightweight sidecar health metadata for the `ping` request.
pub fn ping_result() -> Value {
    json!({
        "healthy": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase0_negotiation_rejects_unknown_required_capabilities_only() {
        let result = negotiate_capabilities(
            &["startExecution".to_string(), "futureRequired".to_string()],
            &[
                "getExecutionEvents".to_string(),
                "futureOptional".to_string(),
            ],
        );
        assert_eq!(result["compatible"], false);
        assert_eq!(result["enabledRequired"], json!(["startExecution"]));
        assert_eq!(result["unsupportedRequired"], json!(["futureRequired"]));
        assert_eq!(result["enabledOptional"], json!(["getExecutionEvents"]));
        assert_eq!(result["unsupportedOptional"], json!(["futureOptional"]));
        assert_eq!(result["extension"]["id"], PHASE0_EXTENSION_ID);
    }
}
