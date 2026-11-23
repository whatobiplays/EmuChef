use std::collections::HashSet;

use ring::hmac;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::errors::ApiError;

const CONTROL_VERSION: u64 = 1;
const CONTROL_DOMAIN: &[u8] = b"EmuChef/TauriSidecar/TrustedControl/v1\0";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustedControlEnvelope {
    version: u64,
    runtime_generation: u64,
    sequence: u64,
    #[serde(default)]
    claims: Option<Value>,
    mac: String,
}

pub struct VerifiedTrustedControl {
    pub claims: Option<Value>,
    runtime_generation: Option<u64>,
    authenticated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeConfigurationWriteClaim {
    pub version: u8,
    pub purpose: String,
    pub runtime_generation: u64,
    pub nonce: String,
    pub review_handle: String,
    pub plan_digest: String,
    pub device_handle: String,
    pub device_serial: String,
    pub session_epoch: u64,
    pub qualification_context_digest: String,
    pub profile_id: String,
    pub target_identity: Value,
    pub destination: String,
    pub operation: String,
    pub source_sha256: String,
    pub original_file_precondition: Value,
    pub backup_mode: String,
    pub backup_waiver_digest: Option<String>,
}

impl VerifiedTrustedControl {
    pub fn without_claims() -> Self {
        Self {
            claims: None,
            runtime_generation: None,
            authenticated: false,
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    pub fn runtime_generation(&self) -> Option<u64> {
        self.runtime_generation
    }
}

pub struct TrustedControlVerifier {
    key: hmac::Key,
    runtime_generation: u64,
    next_sequence: u64,
    used_configuration_nonces: HashSet<String>,
}

impl TrustedControlVerifier {
    pub fn new(key: [u8; 32], runtime_generation: u64) -> Self {
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, &key),
            runtime_generation,
            next_sequence: 1,
            used_configuration_nonces: HashSet::new(),
        }
    }

    pub fn verify_request(
        &mut self,
        request: &mut Value,
    ) -> Result<VerifiedTrustedControl, ApiError> {
        let object = request.as_object_mut().ok_or_else(authentication_error)?;
        let raw_control = object
            .get("trustedControl")
            .cloned()
            .ok_or_else(authentication_error)?;
        let control: TrustedControlEnvelope =
            serde_json::from_value(raw_control).map_err(|_| authentication_error())?;
        if control.version != CONTROL_VERSION
            || control.runtime_generation != self.runtime_generation
            || control.sequence != self.next_sequence
            || control.sequence == 0
        {
            return Err(authentication_error());
        }

        let request_id = object
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(authentication_error)?
            .to_string();
        let request_type = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(authentication_error)?
            .to_string();
        let payload = object.get("payload").unwrap_or(&Value::Null);
        let input = control_mac_input(
            control.runtime_generation,
            control.sequence,
            &request_id,
            &request_type,
            payload,
            control.claims.as_ref(),
        );
        let signature = hex::decode(&control.mac).map_err(|_| authentication_error())?;
        hmac::verify(&self.key, &input, &signature).map_err(|_| authentication_error())?;

        let next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or_else(authentication_error)?;
        object.remove("trustedControl");
        self.next_sequence = next_sequence;

        if let Some(claim) = control.claims.as_ref() {
            let nonce = claim
                .get("nonce")
                .and_then(Value::as_str)
                .filter(|value| is_lower_hex(value, 32))
                .ok_or_else(authentication_error)?;
            if request_type != "startExecution"
                || claim.get("purpose").and_then(Value::as_str)
                    != Some("retroarch_configuration_write")
                || !self.used_configuration_nonces.insert(nonce.to_string())
            {
                return Err(authentication_error());
            }
        }
        Ok(VerifiedTrustedControl {
            claims: control.claims,
            runtime_generation: Some(control.runtime_generation),
            authenticated: true,
        })
    }
}

pub fn reject_unconfigured_control(request: &Value) -> Result<VerifiedTrustedControl, ApiError> {
    if request
        .as_object()
        .is_some_and(|object| object.contains_key("trustedControl"))
    {
        return Err(authentication_error());
    }
    Ok(VerifiedTrustedControl::without_claims())
}

pub fn authentication_error() -> ApiError {
    ApiError::invalid_request_with_details(
        "Private sidecar request authentication failed.",
        json!({ "reason": "trusted_control_authentication_failed" }),
    )
}

pub fn control_mac_input(
    runtime_generation: u64,
    sequence: u64,
    request_id: &str,
    request_type: &str,
    payload: &Value,
    claims: Option<&Value>,
) -> Vec<u8> {
    let mut input = Vec::new();
    input.extend_from_slice(CONTROL_DOMAIN);
    input.extend_from_slice(&runtime_generation.to_be_bytes());
    input.extend_from_slice(&sequence.to_be_bytes());
    append_field(&mut input, request_id.as_bytes());
    append_field(&mut input, request_type.as_bytes());
    append_field(
        &mut input,
        &serde_json::to_vec(payload).expect("JSON values always serialize"),
    );
    append_field(
        &mut input,
        &serde_json::to_vec(&claims).expect("optional JSON values always serialize"),
    );
    input
}

fn append_field(input: &mut Vec<u8>, bytes: &[u8]) {
    input.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    input.extend_from_slice(bytes);
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use ring::hmac;
    use serde_json::{json, Value};

    use super::*;

    const TEST_KEY: [u8; 32] = [0x5a; 32];

    fn signed_request(sequence: u64, payload: Value, claims: Option<Value>) -> Value {
        let request_id = format!("app-{sequence}");
        let request_type = "startExecution";
        let input = control_mac_input(
            7,
            sequence,
            &request_id,
            request_type,
            &payload,
            claims.as_ref(),
        );
        let signature = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &TEST_KEY), &input);
        json!({
            "id": request_id,
            "type": request_type,
            "payload": payload,
            "trustedControl": {
                "version": CONTROL_VERSION,
                "runtimeGeneration": 7,
                "sequence": sequence,
                "claims": claims,
                "mac": hex::encode(signature.as_ref())
            }
        })
    }

    fn configuration_claim(nonce: &str) -> Value {
        json!({
            "purpose": "retroarch_configuration_write",
            "nonce": nonce
        })
    }

    #[test]
    fn verifier_authenticates_request_context_and_removes_private_envelope() {
        let mut verifier = TrustedControlVerifier::new(TEST_KEY, 7);
        let mut request = signed_request(1, json!({ "planDigest": "a" }), None);

        let control = verifier.verify_request(&mut request).unwrap();

        assert!(control.is_authenticated());
        assert_eq!(control.runtime_generation(), Some(7));
        assert!(request.get("trustedControl").is_none());
    }

    #[test]
    fn verifier_rejects_payload_tampering_and_replay() {
        let mut verifier = TrustedControlVerifier::new(TEST_KEY, 7);
        let mut tampered = signed_request(1, json!({ "planDigest": "a" }), None);
        tampered["payload"]["planDigest"] = json!("b");
        assert!(verifier.verify_request(&mut tampered).is_err());

        let mut valid = signed_request(1, json!({ "planDigest": "a" }), None);
        verifier.verify_request(&mut valid).unwrap();
        assert!(verifier.verify_request(&mut valid).is_err());
    }

    #[test]
    fn verifier_rejects_cross_generation_and_reused_operation_nonce() {
        let mut verifier = TrustedControlVerifier::new(TEST_KEY, 7);
        let nonce = "ab".repeat(16);
        let mut first = signed_request(1, json!({}), Some(configuration_claim(&nonce)));
        verifier.verify_request(&mut first).unwrap();

        let mut replayed_claim = signed_request(2, json!({}), Some(configuration_claim(&nonce)));
        assert!(verifier.verify_request(&mut replayed_claim).is_err());

        let mut other_generation = signed_request(2, json!({}), None);
        other_generation["trustedControl"]["runtimeGeneration"] = json!(8);
        assert!(verifier.verify_request(&mut other_generation).is_err());
    }
}
