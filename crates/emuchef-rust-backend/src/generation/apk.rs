//! Safe source facts consumed by the existing app-and-recipe draft generators.
//!
//! These fields are an internal generator input model. APK inspection is owned
//! by `apk_authoring_inspection`; no analyzer identity, certificate evidence,
//! filesystem path, or command output is accepted here.

use serde::{Deserialize, Serialize};

/// Safe facts used to propose authored app and recipe fields.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ApkInspectionFacts {
    pub package_name: Option<String>,
    pub application_label: Option<String>,
    pub version_code: Option<String>,
    pub version_name: Option<String>,
    pub min_sdk: Option<i64>,
    pub target_sdk: Option<i64>,
    pub abis: Vec<String>,
    pub launcher_activities: Vec<String>,
    pub requested_permissions: Vec<String>,
    pub debuggable: Option<bool>,
    pub split: Option<bool>,
    pub base: Option<bool>,
}
