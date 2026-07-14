//! Safe, bounded startup qualification for a copied release application.

use serde_json::{json, Value};

use crate::catalog::CatalogDescriptor;
use crate::sidecar::SidecarState;

/// Returns true only for the exact maintained qualification invocation.
pub(crate) fn requested(arguments: &[String]) -> bool {
    arguments == ["--qualification-probe"]
}

/// Builds the path-free report consumed by the clean-environment smoke test.
fn report(
    runtime_diagnostics: &Value,
    catalog_identity: Value,
    catalog_operation: &Value,
    real_execution_enabled: bool,
) -> Result<Value, ()> {
    let runtime_ready = runtime_diagnostics
        .pointer("/status/status")
        .and_then(Value::as_str)
        == Some("ready");
    let catalog_loaded = catalog_identity
        .pointer("/contentDigest/value")
        .and_then(Value::as_str)
        .is_some_and(|value| value.len() == 64);
    let read_only_catalog_operation = catalog_operation.is_object();
    if !runtime_ready || !catalog_loaded || !read_only_catalog_operation || real_execution_enabled {
        return Err(());
    }
    Ok(json!({
        "kind": "macos_packaged_app_qualification",
        "status": "passed",
        "runtimeReady": true,
        "catalogLoaded": true,
        "readOnlyCatalogOperation": true,
        "realExecutionEnabled": false,
    }))
}

/// Negotiates the normal sidecar and performs one read-only catalog operation.
pub(crate) fn run(
    sidecar: &SidecarState,
    catalog: &CatalogDescriptor,
) -> Result<Value, &'static str> {
    let diagnostics = sidecar.diagnostics();
    if diagnostics
        .pointer("/status/status")
        .and_then(Value::as_str)
        != Some("ready")
    {
        return Err("runtime_not_ready");
    }
    let catalog_operation = sidecar
        .request(
            "describeCatalog",
            json!({ "catalog": catalog.internal_payload() }),
        )
        .map_err(|_| "catalog_operation_failed")?;
    let identity =
        serde_json::to_value(catalog.public_identity()).map_err(|_| "catalog_identity_failed")?;
    report(
        &diagnostics,
        identity,
        &catalog_operation,
        cfg!(feature = "real-execution"),
    )
    .map_err(|_| "qualification_policy_failed")
}

/// Returns a fixed failure envelope that cannot expose paths or sidecar data.
pub(crate) fn failure_report(code: &'static str) -> Value {
    json!({
        "kind": "macos_packaged_app_qualification",
        "status": "failed",
        "code": code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_runtime() -> Value {
        json!({ "status": { "status": "ready", "protocolVersion": 1 } })
    }

    fn identity() -> Value {
        json!({ "contentDigest": { "algorithm": "sha256", "value": "a".repeat(64) } })
    }

    #[test]
    fn only_exact_probe_argument_selects_qualification() {
        assert!(requested(&["--qualification-probe".to_string()]));
        assert!(!requested(&[]));
        assert!(!requested(&[
            "--qualification-probe".to_string(),
            "extra".to_string(),
        ]));
    }

    #[test]
    fn report_requires_ready_runtime_catalog_and_default_disabled_execution() {
        let value = report(
            &ready_runtime(),
            identity(),
            &json!({ "catalog": [] }),
            false,
        )
        .unwrap();
        assert_eq!(value["status"], "passed");
        assert_eq!(value["realExecutionEnabled"], false);
        assert!(report(&json!({}), identity(), &json!({}), false).is_err());
        assert!(report(&ready_runtime(), json!({}), &json!({}), false).is_err());
        assert!(report(&ready_runtime(), identity(), &Value::Null, false).is_err());
        assert!(report(&ready_runtime(), identity(), &json!({}), true).is_err());
    }

    #[test]
    fn failure_report_contains_no_internal_details() {
        assert_eq!(
            failure_report("runtime_not_ready"),
            json!({
                "kind": "macos_packaged_app_qualification",
                "status": "failed",
                "code": "runtime_not_ready",
            })
        );
    }
}
