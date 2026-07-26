//! Session-scoped opaque handles for devices and immutable reviewed plans.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::adb::AdbInstallationIdentity;

const MAX_REVIEWS: usize = 16;
const MAX_TOMBSTONES: usize = 64;
const MAX_DEVICE_IDENTITIES: usize = 32;
const REVIEW_IDLE_TTL: Duration = Duration::from_secs(30 * 60);
const REVIEW_ABSOLUTE_TTL: Duration = Duration::from_secs(2 * 60 * 60);

#[derive(Clone, Debug)]
pub struct DeviceRecord {
    pub handle: String,
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
    pub facts: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDto {
    device_handle: String,
    state: String,
    display_name: String,
    masked_serial: String,
}

#[derive(Clone, Debug)]
pub struct ReviewedPlanSnapshot {
    pub response: Value,
    pub target: Value,
    pub catalog_identity: Value,
    pub catalog_digest: String,
    pub plan_digest: String,
    pub device_handle: String,
    pub platform_tools_identity: Option<AdbInstallationIdentity>,
    pub created: Instant,
    pub last_access: Instant,
}

#[derive(Clone, Debug)]
struct ReviewTombstone {
    handle: String,
    code: &'static str,
}

#[derive(Default)]
pub struct SessionHandles {
    devices_by_handle: HashMap<String, DeviceRecord>,
    handle_by_serial: HashMap<String, String>,
    identity_order: VecDeque<String>,
    reviews: HashMap<String, ReviewedPlanSnapshot>,
    review_order: VecDeque<String>,
    tombstones: VecDeque<ReviewTombstone>,
    device_generation: u64,
}

impl SessionHandles {
    pub fn update_devices(&mut self, inventory: &Value) -> Result<Vec<DeviceDto>, String> {
        let raw = inventory
            .get("devices")
            .and_then(Value::as_array)
            .ok_or_else(|| "ADB inventory response was invalid.".to_string())?;
        let mut present = HashMap::new();
        for device in raw {
            let serial = device
                .get("serial")
                .and_then(Value::as_str)
                .ok_or_else(|| "ADB inventory device omitted its serial.".to_string())?;
            let state = device
                .get("state")
                .and_then(Value::as_str)
                .ok_or_else(|| "ADB inventory device omitted its state.".to_string())?;
            let model = device
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string);
            let handle = self.handle_for_serial(serial);
            let facts = self
                .devices_by_handle
                .get(&handle)
                .and_then(|record| record.facts.clone());
            present.insert(
                handle.clone(),
                DeviceRecord {
                    handle: handle.clone(),
                    serial: serial.to_string(),
                    state: state.to_string(),
                    model,
                    facts,
                },
            );
        }
        let disappeared = self
            .devices_by_handle
            .keys()
            .filter(|handle| !present.contains_key(*handle))
            .cloned()
            .collect::<Vec<_>>();
        for handle in disappeared {
            self.invalidate_reviews_for_device(&handle, "review_stale");
        }
        self.devices_by_handle = present;
        self.device_generation = self.device_generation.saturating_add(1).max(1);
        let mut result = self
            .devices_by_handle
            .values()
            .map(|record| DeviceDto {
                device_handle: record.handle.clone(),
                state: record.state.clone(),
                display_name: record
                    .model
                    .as_deref()
                    .map(|model| model.replace(&record.serial, "Android device"))
                    .unwrap_or_else(|| "Android device".to_string()),
                masked_serial: mask_serial(&record.serial),
            })
            .collect::<Vec<_>>();
        result.sort_by(|left, right| left.device_handle.cmp(&right.device_handle));
        Ok(result)
    }

    /// Aggregate, serial-free device facts for troubleshooting projection.
    pub fn support_summary(&self) -> Value {
        let available = self
            .devices_by_handle
            .values()
            .filter(|device| device.state == "available")
            .count();
        let unauthorized = self
            .devices_by_handle
            .values()
            .filter(|device| device.state == "unauthorized")
            .count();
        let offline = self
            .devices_by_handle
            .values()
            .filter(|device| device.state == "offline")
            .count();
        json!({
            "generation": self.device_generation,
            "deviceCount": self.devices_by_handle.len(),
            "availableCount": available,
            "unauthorizedCount": unauthorized,
            "offlineCount": offline,
        })
    }

    pub fn device_generation(&self) -> u64 {
        self.device_generation
    }

    /// Return the trusted native device records used to resolve qualification
    /// requests. Exact serials never cross the React projection boundary.
    pub fn qualification_devices(&self) -> Vec<DeviceRecord> {
        let mut devices = self.devices_by_handle.values().cloned().collect::<Vec<_>>();
        devices.sort_by(|left, right| left.handle.cmp(&right.handle));
        devices
    }

    pub fn single_available_device_handle(&self) -> Option<String> {
        if self.devices_by_handle.len() != 1 {
            return None;
        }
        self.devices_by_handle
            .values()
            .next()
            .filter(|device| device.state == "available")
            .map(|device| device.handle.clone())
    }

    pub fn device(&self, handle: &str) -> Result<&DeviceRecord, String> {
        self.devices_by_handle.get(handle).ok_or_else(|| {
            stable_handle_error("device_unknown", "This device is no longer available.")
        })
    }

    pub fn set_facts(&mut self, handle: &str, facts: Value) -> Result<(), String> {
        let changed = self
            .devices_by_handle
            .get(handle)
            .ok_or_else(|| {
                stable_handle_error("device_unknown", "This device is no longer available.")
            })?
            .facts
            .as_ref()
            .is_some_and(|current| current != &facts);
        if changed {
            self.invalidate_reviews_for_device(handle, "review_stale");
        }
        self.devices_by_handle
            .get_mut(handle)
            .expect("device was checked above")
            .facts = Some(facts);
        Ok(())
    }

    pub fn facts(&self, handle: &str) -> Result<&Value, String> {
        self.device(handle)?.facts.as_ref().ok_or_else(|| {
            stable_handle_error("device_not_probed", "Read the device information again.")
        })
    }

    pub fn insert_review(&mut self, mut snapshot: ReviewedPlanSnapshot) -> String {
        self.expire_reviews();
        while self.reviews.len() >= MAX_REVIEWS {
            if let Some(oldest) = self.review_order.pop_front() {
                self.reviews.remove(&oldest);
                self.push_tombstone(oldest, "review_stale");
            }
        }
        let handle = format!("review_{}", Uuid::new_v4().simple());
        let now = Instant::now();
        snapshot.created = now;
        snapshot.last_access = now;
        self.review_order.push_back(handle.clone());
        self.reviews.insert(handle.clone(), snapshot);
        handle
    }

    pub fn review(&mut self, handle: &str) -> Result<&ReviewedPlanSnapshot, String> {
        self.expire_reviews();
        if let Some(tombstone) = self.tombstones.iter().find(|item| item.handle == handle) {
            return Err(stable_handle_error(
                tombstone.code,
                "This reviewed plan is no longer valid. Generate a new review.",
            ));
        }
        let review = self.reviews.get_mut(handle).ok_or_else(|| {
            stable_handle_error("review_unknown", "This reviewed plan handle is unknown.")
        })?;
        review.last_access = Instant::now();
        Ok(review)
    }

    pub fn discard_review(&mut self, handle: &str) -> Result<(), String> {
        self.expire_reviews();
        if self.reviews.remove(handle).is_some() {
            self.review_order.retain(|candidate| candidate != handle);
            self.push_tombstone(handle.to_string(), "review_stale");
            Ok(())
        } else if let Some(tombstone) = self.tombstones.iter().find(|item| item.handle == handle) {
            Err(stable_handle_error(
                tombstone.code,
                "This reviewed plan is no longer valid. Generate a new review.",
            ))
        } else {
            Err(stable_handle_error(
                "review_unknown",
                "This reviewed plan handle is unknown.",
            ))
        }
    }

    /// Invalidates one retained review without disturbing any unrelated review.
    pub fn invalidate_review(&mut self, handle: &str, code: &'static str) {
        if self.reviews.remove(handle).is_some() {
            self.review_order.retain(|candidate| candidate != handle);
            self.push_tombstone(handle.to_string(), code);
        }
    }

    pub fn invalidate_all(&mut self) {
        self.invalidate_runtime_authority_preserving_identities();
        self.handle_by_serial.clear();
        self.identity_order.clear();
    }

    /// Clears live device and review authority while retaining only bounded,
    /// process-local identity continuity for controlled redetection.
    pub fn invalidate_runtime_authority_preserving_identities(&mut self) {
        let handles = self.reviews.keys().cloned().collect::<Vec<_>>();
        for handle in handles {
            self.reviews.remove(&handle);
            self.push_tombstone(handle, "review_stale");
        }
        self.review_order.clear();
        self.devices_by_handle.clear();
    }

    pub fn invalidate_catalog(&mut self, current_digest: &str) {
        let handles = self
            .reviews
            .iter()
            .filter(|(_, review)| review.catalog_digest != current_digest)
            .map(|(handle, _)| handle.clone())
            .collect::<Vec<_>>();
        for handle in handles {
            self.reviews.remove(&handle);
            self.review_order.retain(|candidate| candidate != &handle);
            self.push_tombstone(handle, "review_stale");
        }
    }

    pub fn invalidate_reviews_for_device(&mut self, device_handle: &str, code: &'static str) {
        let handles = self
            .reviews
            .iter()
            .filter(|(_, review)| review.device_handle == device_handle)
            .map(|(handle, _)| handle.clone())
            .collect::<Vec<_>>();
        for handle in handles {
            self.reviews.remove(&handle);
            self.review_order.retain(|candidate| candidate != &handle);
            self.push_tombstone(handle, code);
        }
    }

    fn expire_reviews(&mut self) {
        let now = Instant::now();
        let expired = self
            .reviews
            .iter()
            .filter(|(_, review)| {
                now.duration_since(review.last_access) >= REVIEW_IDLE_TTL
                    || now.duration_since(review.created) >= REVIEW_ABSOLUTE_TTL
            })
            .map(|(handle, _)| handle.clone())
            .collect::<Vec<_>>();
        for handle in expired {
            self.reviews.remove(&handle);
            self.review_order.retain(|candidate| candidate != &handle);
            self.push_tombstone(handle, "review_expired");
        }
    }

    fn push_tombstone(&mut self, handle: String, code: &'static str) {
        if self.tombstones.len() >= MAX_TOMBSTONES {
            self.tombstones.pop_front();
        }
        self.tombstones.push_back(ReviewTombstone { handle, code });
    }

    fn handle_for_serial(&mut self, serial: &str) -> String {
        if let Some(handle) = self.handle_by_serial.get(serial).cloned() {
            self.identity_order.retain(|candidate| candidate != serial);
            self.identity_order.push_back(serial.to_string());
            return handle;
        }
        while self.handle_by_serial.len() >= MAX_DEVICE_IDENTITIES {
            let Some(oldest) = self.identity_order.pop_front() else {
                break;
            };
            self.handle_by_serial.remove(&oldest);
        }
        let handle = format!("device_{}", Uuid::new_v4().simple());
        self.handle_by_serial
            .insert(serial.to_string(), handle.clone());
        self.identity_order.push_back(serial.to_string());
        handle
    }
}

fn mask_serial(serial: &str) -> String {
    let tail = serial.chars().rev().take(4).collect::<Vec<_>>();
    if serial.chars().count() <= 4 {
        return "••••".to_string();
    }
    format!("••••-{}", tail.into_iter().rev().collect::<String>())
}

fn stable_handle_error(code: &str, message: &str) -> String {
    json!({ "code": code, "message": message }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_snapshot(device_handle: &str) -> ReviewedPlanSnapshot {
        ReviewedPlanSnapshot {
            response: json!({ "plan": { "id": "plan" } }),
            target: json!({ "serial": "trusted" }),
            catalog_identity: json!({ "sourceId": "catalog" }),
            catalog_digest: "sha256:catalog".to_string(),
            plan_digest: "sha256:plan".to_string(),
            device_handle: device_handle.to_string(),
            platform_tools_identity: None,
            created: Instant::now(),
            last_access: Instant::now(),
        }
    }

    #[test]
    fn polling_reuses_handles_and_never_serializes_exact_serial() {
        let mut store = SessionHandles::default();
        let inventory = json!({ "devices": [{ "serial": "sensitive-1234", "state": "available", "model": "Pocket" }] });
        let first = store.update_devices(&inventory).unwrap();
        let second = store.update_devices(&inventory).unwrap();
        assert_eq!(first[0].device_handle, second[0].device_handle);
        let serialized = serde_json::to_string(&first).unwrap();
        assert!(!serialized.contains("sensitive"));
        assert!(serialized.contains("1234"));
    }

    #[test]
    fn qualification_devices_are_native_records_sorted_by_opaque_handle() {
        let mut store = SessionHandles::default();
        store
            .update_devices(&json!({
                "devices": [
                    { "serial": "second", "state": "offline" },
                    { "serial": "first", "state": "available" }
                ]
            }))
            .unwrap();

        let records = store.qualification_devices();
        assert_eq!(records.len(), 2);
        assert!(records
            .windows(2)
            .all(|pair| pair[0].handle <= pair[1].handle));
        assert!(records.iter().any(|record| record.serial == "first"));
    }

    #[test]
    fn disappearance_invalidates_authority_but_retains_session_identity() {
        let mut store = SessionHandles::default();
        let devices = store
            .update_devices(&json!({ "devices": [{ "serial": "one", "state": "available" }] }))
            .unwrap();
        let handle = devices[0].device_handle.clone();
        store.update_devices(&json!({ "devices": [] })).unwrap();
        assert!(store
            .device(&handle)
            .unwrap_err()
            .contains("device_unknown"));
        let reappeared = store
            .update_devices(&json!({ "devices": [{ "serial": "one", "state": "available" }] }))
            .unwrap();
        assert_eq!(reappeared[0].device_handle, handle);
    }

    #[test]
    fn authorization_transition_reuses_the_handle_and_becomes_available() {
        let mut store = SessionHandles::default();
        let unauthorized = store
            .update_devices(&json!({ "devices": [{ "serial": "one", "state": "unauthorized" }] }))
            .unwrap();
        let authorized = store
            .update_devices(&json!({ "devices": [{ "serial": "one", "state": "available" }] }))
            .unwrap();
        assert_eq!(unauthorized[0].device_handle, authorized[0].device_handle);
        assert_eq!(unauthorized[0].state, "unauthorized");
        assert_eq!(authorized[0].state, "available");
    }

    #[test]
    fn controlled_invalidation_preserves_identity_but_full_reset_does_not() {
        let mut store = SessionHandles::default();
        let original = store
            .update_devices(&json!({ "devices": [{ "serial": "one", "state": "available" }] }))
            .unwrap()[0]
            .device_handle
            .clone();
        store.invalidate_runtime_authority_preserving_identities();
        let redetected = store
            .update_devices(&json!({ "devices": [{ "serial": "one", "state": "available" }] }))
            .unwrap()[0]
            .device_handle
            .clone();
        assert_eq!(redetected, original);
        store.invalidate_all();
        let after_reset = store
            .update_devices(&json!({ "devices": [{ "serial": "one", "state": "available" }] }))
            .unwrap()[0]
            .device_handle
            .clone();
        assert_ne!(after_reset, original);
    }

    #[test]
    fn reconnect_identity_cache_is_bounded() {
        let mut store = SessionHandles::default();
        let first = store
            .update_devices(&json!({ "devices": [{ "serial": "serial-0", "state": "available" }] }))
            .unwrap()[0]
            .device_handle
            .clone();
        for index in 1..=MAX_DEVICE_IDENTITIES {
            store
                .update_devices(&json!({ "devices": [{
                    "serial": format!("serial-{index}"),
                    "state": "available"
                }] }))
                .unwrap();
        }
        assert_eq!(store.handle_by_serial.len(), MAX_DEVICE_IDENTITIES);
        let evicted = store
            .update_devices(&json!({ "devices": [{ "serial": "serial-0", "state": "available" }] }))
            .unwrap()[0]
            .device_handle
            .clone();
        assert_ne!(evicted, first);
    }

    #[test]
    fn unknown_review_has_stable_code() {
        let mut store = SessionHandles::default();
        assert!(store
            .review("review_missing")
            .unwrap_err()
            .contains("review_unknown"));
    }

    #[test]
    fn review_expiration_and_discard_use_stable_codes() {
        let mut store = SessionHandles::default();
        let expired = store.insert_review(review_snapshot("device_one"));
        store.reviews.get_mut(&expired).unwrap().last_access =
            Instant::now() - REVIEW_IDLE_TTL - Duration::from_secs(1);
        assert!(store
            .review(&expired)
            .unwrap_err()
            .contains("review_expired"));

        let absolute = store.insert_review(review_snapshot("device_one"));
        store.reviews.get_mut(&absolute).unwrap().created =
            Instant::now() - REVIEW_ABSOLUTE_TTL - Duration::from_secs(1);
        assert!(store
            .review(&absolute)
            .unwrap_err()
            .contains("review_expired"));

        let discarded = store.insert_review(review_snapshot("device_one"));
        store.discard_review(&discarded).unwrap();
        assert!(store
            .review(&discarded)
            .unwrap_err()
            .contains("review_stale"));
        assert!(store
            .discard_review(&discarded)
            .unwrap_err()
            .contains("review_stale"));

        let discard_expired = store.insert_review(review_snapshot("device_one"));
        store.reviews.get_mut(&discard_expired).unwrap().last_access =
            Instant::now() - REVIEW_IDLE_TTL - Duration::from_secs(1);
        assert!(store
            .discard_review(&discard_expired)
            .unwrap_err()
            .contains("review_expired"));
    }

    #[test]
    fn review_store_is_bounded_and_retains_complete_snapshot() {
        let mut store = SessionHandles::default();
        let first = store.insert_review(review_snapshot("device_one"));
        for _ in 1..=MAX_REVIEWS {
            store.insert_review(review_snapshot("device_one"));
        }
        assert_eq!(store.reviews.len(), MAX_REVIEWS);
        assert!(store.review(&first).unwrap_err().contains("review_stale"));
        let newest = store.review_order.back().unwrap().clone();
        let snapshot = store.review(&newest).unwrap();
        assert_eq!(snapshot.target["serial"], "trusted");
        assert_eq!(snapshot.response["plan"]["id"], "plan");
        assert_eq!(snapshot.catalog_identity["sourceId"], "catalog");
        assert_eq!(snapshot.plan_digest, "sha256:plan");
    }

    #[test]
    fn device_disappearance_stales_bound_reviews() {
        let mut store = SessionHandles::default();
        let devices = store
            .update_devices(&json!({ "devices": [{ "serial": "one", "state": "available" }] }))
            .unwrap();
        let handle = devices[0].device_handle.clone();
        let review = store.insert_review(review_snapshot(&handle));
        store.update_devices(&json!({ "devices": [] })).unwrap();
        assert!(store.review(&review).unwrap_err().contains("review_stale"));
    }

    #[test]
    fn targeted_review_invalidation_preserves_unrelated_reviews() {
        let mut store = SessionHandles::default();
        let invalidated = store.insert_review(review_snapshot("device_one"));
        let retained = store.insert_review(review_snapshot("device_two"));

        store.invalidate_review(&invalidated, "review_stale");

        assert!(store
            .review(&invalidated)
            .unwrap_err()
            .contains("review_stale"));
        assert_eq!(store.review(&retained).unwrap().device_handle, "device_two");
    }

    #[test]
    fn changed_facts_and_catalog_digest_stale_reviews() {
        let mut store = SessionHandles::default();
        let devices = store
            .update_devices(&json!({ "devices": [{ "serial": "one", "state": "available" }] }))
            .unwrap();
        let handle = devices[0].device_handle.clone();
        store.set_facts(&handle, json!({ "model": "one" })).unwrap();
        let changed = store.insert_review(review_snapshot(&handle));
        store.set_facts(&handle, json!({ "model": "two" })).unwrap();
        assert!(store.review(&changed).unwrap_err().contains("review_stale"));

        let catalog = store.insert_review(review_snapshot(&handle));
        store.invalidate_catalog("different-catalog");
        assert!(store.review(&catalog).unwrap_err().contains("review_stale"));
    }

    #[test]
    fn review_tombstones_are_bounded_and_age_to_unknown() {
        let mut store = SessionHandles::default();
        let first = store.insert_review(review_snapshot("device_one"));
        store.discard_review(&first).unwrap();
        for _ in 0..MAX_TOMBSTONES {
            let handle = store.insert_review(review_snapshot("device_one"));
            store.discard_review(&handle).unwrap();
        }
        assert_eq!(store.tombstones.len(), MAX_TOMBSTONES);
        assert!(store.review(&first).unwrap_err().contains("review_unknown"));
    }
}
