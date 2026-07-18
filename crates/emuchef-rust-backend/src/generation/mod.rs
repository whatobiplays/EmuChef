//! Side-effect-free authored-data generation used by trusted editor wrappers.
//!
//! Generators in this module construct the existing typed authored models,
//! validate them through the shared schema authority, and return reviewable
//! drafts. Filesystem publication remains the responsibility of the Tauri host.

mod apk;
mod app_recipe;
mod collisions;
mod device_profile;
mod identifiers;
mod remote_app_recipe;

pub(crate) use app_recipe::{generate_app_recipe_draft, AppRecipeDraftRequest};
pub(crate) use collisions::{
    check_app_recipe_collisions, check_device_profile_collisions, AppRecipeCollisionRequest,
};
pub(crate) use device_profile::{
    generate_device_profile_draft, DeviceProfileDraftRequest, SafeDetectedDeviceFacts,
};
pub(crate) use remote_app_recipe::{generate_remote_app_recipe_draft, RemoteAppRecipeDraftRequest};
