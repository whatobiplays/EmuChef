//! Side-effect-free authored-data generation used by trusted editor wrappers.
//!
//! Generators in this module construct the existing typed authored models,
//! validate them through the shared schema authority, and return reviewable
//! drafts. Filesystem publication remains the responsibility of the Tauri host.

mod collisions;
mod device_profile;

pub(crate) use collisions::check_device_profile_collisions;
pub(crate) use device_profile::{
    generate_device_profile_draft, DeviceProfileDraftRequest, SafeDetectedDeviceFacts,
};
