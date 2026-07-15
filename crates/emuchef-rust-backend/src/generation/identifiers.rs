//! Deterministic authored identifiers shared by guided generators.

/// Normalize one human or package identity component into the schema-v1 grammar.
pub(super) fn normalize_identifier_component(value: &str) -> String {
    let mut normalized = String::new();
    let mut separator_pending = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if separator_pending && !normalized.is_empty() {
                normalized.push('_');
            }
            normalized.push(character.to_ascii_lowercase());
            separator_pending = false;
        } else if !normalized.is_empty() {
            separator_pending = true;
        }
    }
    normalized
}

/// Produce a ref-safe local identifier token from a valid authored app id.
pub(super) fn recipe_local_token(app_id: &str) -> String {
    normalize_identifier_component(app_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_is_lowercase_segmented_and_stable() {
        assert_eq!(
            normalize_identifier_component("RetroArch (AArch64)"),
            "retroarch_aarch64"
        );
        assert_eq!(recipe_local_token("com.example-app"), "com_example_app");
    }
}
