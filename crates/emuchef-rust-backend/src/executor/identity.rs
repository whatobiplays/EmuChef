//! Private same-serial physical-identity evidence for real ADB execution.

use super::adb::{AdbCommandError, AdbCommandExecutor, AdbCommandRunner};
use crate::planner::TargetDeviceBinding;

/// The only marker allowed to influence Tauri's existing partial-change
/// projection. It is deliberately generic and contains no identity evidence.
pub(crate) const POST_OPERATION_IDENTITY_FAILURE_MARKER: &str =
    "The device identity could not be verified after the operation may have run.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IdentityCheckPhase {
    PreOperation,
    PostOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeviceIdentityFingerprint {
    manufacturer: String,
    brand: String,
    model: String,
    product: String,
    device: String,
    board: String,
    hardware: String,
    hardware_sku: Option<String>,
    android_api_level: u32,
    abis: Vec<String>,
    build_fingerprint: String,
    android_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SampleComparison {
    Same,
    Changed,
    Unverified,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct IdentityGuard {
    enabled: bool,
    target: Option<ReviewedIdentityTarget>,
    baseline: Option<DeviceIdentityFingerprint>,
}

#[derive(Clone, Debug)]
struct IdentityTarget {
    serial: String,
    manufacturer: String,
    model: String,
    android_api_level: u32,
}

#[derive(Clone, Debug)]
enum ReviewedIdentityTarget {
    Ready(IdentityTarget),
    Insufficient,
}

impl IdentityGuard {
    pub(crate) fn disable(&mut self) {
        self.enabled = false;
        self.target = None;
        self.baseline = None;
    }

    pub(crate) fn configure(&mut self, target: Option<&TargetDeviceBinding>) {
        self.enabled = true;
        self.target = Some(match target {
            Some(target) => {
                let serial = target.serial.trim();
                let manufacturer = target.manufacturer.as_deref().and_then(normalize_text);
                let model = target.model.as_deref().and_then(normalize_text);
                let android_api_level = target
                    .android_api_level
                    .and_then(|value| u32::try_from(value).ok())
                    .filter(|value| *value > 0);
                match (serial.is_empty(), manufacturer, model, android_api_level) {
                    (false, Some(manufacturer), Some(model), Some(android_api_level)) => {
                        ReviewedIdentityTarget::Ready(IdentityTarget {
                            serial: serial.to_string(),
                            manufacturer,
                            model,
                            android_api_level,
                        })
                    }
                    _ => ReviewedIdentityTarget::Insufficient,
                }
            }
            None => ReviewedIdentityTarget::Insufficient,
        });
        self.baseline = None;
    }

    pub(crate) fn check<E: AdbCommandExecutor>(
        &mut self,
        runner: &mut AdbCommandRunner<E>,
        phase: IdentityCheckPhase,
    ) -> Result<(), AdbCommandError> {
        if !self.enabled {
            return Ok(());
        }
        let Some(ReviewedIdentityTarget::Ready(target)) = self.target.as_ref() else {
            return Err(AdbCommandError::DeviceIdentityUnverified { phase });
        };
        if runner.serial().is_none() {
            return Err(AdbCommandError::DeviceIdentityUnverified { phase });
        }
        if runner.serial() != Some(target.serial.as_str()) {
            return Err(AdbCommandError::DeviceIdentityChanged { phase });
        }
        let Some(fingerprint) = collect_fingerprint(runner)? else {
            return Err(AdbCommandError::DeviceIdentityUnverified { phase });
        };

        let Some(baseline) = self.baseline.as_ref() else {
            if !matches_reviewed_target(&fingerprint, target) {
                return Err(AdbCommandError::DeviceIdentityChanged { phase });
            }
            self.baseline = Some(fingerprint);
            return Ok(());
        };

        match compare_samples(baseline, &fingerprint) {
            SampleComparison::Same => Ok(()),
            SampleComparison::Changed => Err(AdbCommandError::DeviceIdentityChanged { phase }),
            SampleComparison::Unverified => {
                Err(AdbCommandError::DeviceIdentityUnverified { phase })
            }
        }
    }
}

fn collect_fingerprint<E: AdbCommandExecutor>(
    runner: &mut AdbCommandRunner<E>,
) -> Result<Option<DeviceIdentityFingerprint>, AdbCommandError> {
    let Some(first) = collect_sample(runner)? else {
        return Ok(None);
    };
    let Some(second) = collect_sample(runner)? else {
        return Ok(None);
    };
    if compare_samples(&first, &second) != SampleComparison::Same {
        return Ok(None);
    }
    Ok(Some(first))
}

fn collect_sample<E: AdbCommandExecutor>(
    runner: &mut AdbCommandRunner<E>,
) -> Result<Option<DeviceIdentityFingerprint>, AdbCommandError> {
    let getprop =
        match runner.run_identity_command(vec!["shell".to_string(), "getprop".to_string()]) {
            Ok(result) => result,
            Err(AdbCommandError::CommandFailed) => return Ok(None),
            Err(error) => return Err(error),
        };
    let Some(properties) = parse_getprop_sample(&getprop.stdout) else {
        return Ok(None);
    };
    let android_id = match runner.run_identity_command(vec![
        "shell".to_string(),
        "settings".to_string(),
        "get".to_string(),
        "secure".to_string(),
        "android_id".to_string(),
    ]) {
        Ok(result) => result,
        Err(AdbCommandError::CommandFailed) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(android_id) = parse_android_id_output(&android_id.stdout) else {
        return Ok(None);
    };
    Ok(properties.into_fingerprint(android_id))
}

fn matches_reviewed_target(
    fingerprint: &DeviceIdentityFingerprint,
    target: &IdentityTarget,
) -> bool {
    target.manufacturer == fingerprint.manufacturer
        && target.model == fingerprint.model
        && target.android_api_level == fingerprint.android_api_level
}

fn compare_samples(
    first: &DeviceIdentityFingerprint,
    second: &DeviceIdentityFingerprint,
) -> SampleComparison {
    if first.hardware_sku.is_some() != second.hardware_sku.is_some() {
        return SampleComparison::Unverified;
    }
    if first == second {
        SampleComparison::Same
    } else {
        SampleComparison::Changed
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ParsedGetpropSample {
    manufacturer: Option<String>,
    brand: Option<String>,
    model: Option<String>,
    product: Option<String>,
    device: Option<String>,
    board: Option<String>,
    hardware: Option<String>,
    hardware_sku: Option<String>,
    android_api_level: Option<u32>,
    abis: Option<Vec<String>>,
    fallback_abis: Vec<String>,
    build_fingerprint: Option<String>,
}

impl ParsedGetpropSample {
    fn into_fingerprint(self, android_id: String) -> Option<DeviceIdentityFingerprint> {
        let abis = self
            .abis
            .filter(|abis| !abis.is_empty())
            .unwrap_or_else(|| normalize_abis(self.fallback_abis));
        if abis.is_empty() {
            return None;
        }
        Some(DeviceIdentityFingerprint {
            manufacturer: self.manufacturer?,
            brand: self.brand?,
            model: self.model?,
            product: self.product?,
            device: self.device?,
            board: self.board?,
            hardware: self.hardware?,
            hardware_sku: self.hardware_sku,
            android_api_level: self.android_api_level?,
            abis,
            build_fingerprint: self.build_fingerprint?,
            android_id,
        })
    }
}

fn parse_getprop_sample(stdout: &str) -> Option<ParsedGetpropSample> {
    let mut sample = ParsedGetpropSample::default();
    for line in stdout.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((key, value)) = rest.split_once("]: [") else {
            continue;
        };
        let Some(value) = value.strip_suffix(']') else {
            continue;
        };
        let raw_value = value;
        let value = normalize_text(raw_value);
        match key {
            "ro.product.manufacturer" => set_once(&mut sample.manufacturer, value)?,
            "ro.product.brand" => set_once(&mut sample.brand, value)?,
            "ro.product.model" => set_once(&mut sample.model, value)?,
            "ro.product.name" => set_once(&mut sample.product, value)?,
            "ro.product.device" => set_once(&mut sample.device, value)?,
            "ro.product.board" => set_once(&mut sample.board, value)?,
            "ro.hardware" => set_once(&mut sample.hardware, value)?,
            "ro.boot.hardware.sku" | "ro.product.hardware.sku" => {
                set_once(&mut sample.hardware_sku, value)?
            }
            "ro.build.version.sdk" => {
                let value = value?.parse::<u32>().ok().filter(|value| *value > 0);
                set_once(&mut sample.android_api_level, value)?;
            }
            "ro.product.cpu.abilist" => set_once(
                &mut sample.abis,
                value.map(|value| normalize_abis(value.split(','))),
            )?,
            "ro.product.cpu.abi" | "ro.product.cpu.abi2" => {
                if let Some(value) = value {
                    sample.fallback_abis.push(value);
                }
            }
            "ro.build.fingerprint" => set_once(
                &mut sample.build_fingerprint,
                normalize_case_sensitive_text(raw_value),
            )?,
            _ => {}
        }
    }
    Some(sample)
}

fn set_once<T: PartialEq>(slot: &mut Option<T>, value: Option<T>) -> Option<()> {
    if let Some(existing) = slot.as_ref() {
        if value.as_ref() != Some(existing) {
            return None;
        }
    } else {
        *slot = value;
    }
    Some(())
}

fn normalize_abis(values: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<String> {
    values
        .into_iter()
        .filter_map(|value| normalize_text(value.as_ref()))
        .map(|value| value.to_ascii_lowercase())
        .collect()
}

fn normalize_text(value: &str) -> Option<String> {
    normalize_text_with_case(value, true)
}

fn normalize_case_sensitive_text(value: &str) -> Option<String> {
    normalize_text_with_case(value, false)
}

fn normalize_text_with_case(value: &str, lowercase: bool) -> Option<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty()
        || matches!(normalized.to_ascii_lowercase().as_str(), "null" | "unknown")
    {
        return None;
    }
    Some(if lowercase {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    })
}

fn normalize_android_id(value: &str) -> Option<String> {
    let normalized = normalize_text(value)?;
    if normalized == "9774d56d682e549c"
        || normalized.len() > 16
        || !normalized
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return None;
    }
    let parsed = u64::from_str_radix(&normalized, 16).ok()?;
    (parsed != 0).then_some(normalized)
}

fn parse_android_id_output(stdout: &str) -> Option<String> {
    let mut lines = stdout.lines().filter(|line| !line.trim().is_empty());
    let value = lines.next()?.trim();
    if lines.next().is_some() {
        return None;
    }
    normalize_android_id(value)
}

#[cfg(test)]
fn fingerprint_for_test(model: &str, hardware_sku: Option<&str>) -> DeviceIdentityFingerprint {
    DeviceIdentityFingerprint {
        manufacturer: "acme".to_string(),
        brand: "acme".to_string(),
        model: model.to_ascii_lowercase(),
        product: "pocket".to_string(),
        device: "pocket".to_string(),
        board: "board".to_string(),
        hardware: "hardware".to_string(),
        hardware_sku: hardware_sku.map(str::to_string),
        android_api_level: 33,
        abis: vec!["arm64-v8a".to_string()],
        build_fingerprint: "acme/pocket/pocket:13/TQ1A:user/release-keys".to_string(),
        android_id: "a1b2".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::adb::FakeAdbCommandExecutor;
    use crate::owned_process::{ProcessFailureKind, ProcessOperation};
    use crate::planner::TargetDeviceBinding;

    #[derive(Clone, Copy)]
    enum FailureScript {
        Timeout,
        Process,
        Transport,
    }

    #[test]
    fn android_id_accepts_short_hex_and_rejects_invalid_or_placeholder_values() {
        assert_eq!(normalize_android_id("  A1b2  "), Some("a1b2".to_string()));
        for value in [
            "",
            " ",
            "null",
            "unknown",
            "0000",
            "0000000000000000",
            "9774d56d682e549c",
            "not-hex",
            "0123456789abcdef0",
        ] {
            assert_eq!(
                normalize_android_id(value),
                None,
                "{value:?} should be rejected"
            );
        }
    }

    #[test]
    fn complete_samples_canonicalize_independently_of_property_order_and_whitespace() {
        let first = parse_getprop_sample(
            "[ro.product.model]: [ Pocket  S ]\n[ro.product.manufacturer]: [ Acme ]\n",
        );
        let second = parse_getprop_sample(
            "[ro.product.manufacturer]: [Acme]\n[ro.product.model]: [Pocket S]\n",
        );
        assert_eq!(first, second);
    }

    #[test]
    fn build_fingerprint_comparison_preserves_meaningful_case() {
        let upper = parse_getprop_sample(&complete_getprop("Pocket S", "Acme/FP-A"))
            .and_then(|sample| sample.into_fingerprint("a1b2".to_string()))
            .expect("complete sample should parse");
        let lower = parse_getprop_sample(&complete_getprop("Pocket S", "acme/fp-a"))
            .and_then(|sample| sample.into_fingerprint("a1b2".to_string()))
            .expect("complete sample should parse");
        assert_eq!(compare_samples(&upper, &lower), SampleComparison::Changed);
    }

    #[test]
    fn stable_value_changes_are_identity_mismatches_but_optional_presence_changes_are_unverified() {
        let first = fingerprint_for_test("Pocket S", Some("sku-a"));
        let changed = fingerprint_for_test("Pocket X", Some("sku-a"));
        let optional_changed = fingerprint_for_test("Pocket S", None);
        assert_eq!(compare_samples(&first, &changed), SampleComparison::Changed);
        assert_eq!(
            compare_samples(&first, &optional_changed),
            SampleComparison::Unverified
        );
    }

    #[test]
    fn serial_and_transport_id_are_not_part_of_the_private_fingerprint() {
        let first = fingerprint_for_test("Pocket S", Some("sku-a"));
        let mut second = first.clone();
        second.android_id = first.android_id.clone();
        assert_eq!(compare_samples(&first, &second), SampleComparison::Same);
    }

    #[test]
    fn successful_collection_uses_four_bounded_probe_commands_and_no_root_command() {
        let mut executor = FakeAdbCommandExecutor::default();
        push_complete_sample(&mut executor, "a1b2");
        push_complete_sample(&mut executor, "a1b2");
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("serial-a".to_string()), executor);
        let mut guard = IdentityGuard::default();
        guard.configure(Some(&target("acme", "Pocket S", 33)));

        guard
            .check(&mut runner, IdentityCheckPhase::PreOperation)
            .unwrap();

        assert_eq!(runner.executor().calls().len(), 4);
        assert!(runner
            .executor()
            .operations()
            .iter()
            .all(|operation| *operation == ProcessOperation::Probe));
        assert!(runner
            .executor()
            .calls()
            .iter()
            .all(|call| call.windows(2).any(|pair| pair == ["-s", "serial-a"])));
    }

    #[test]
    fn missing_android_id_or_inconsistent_samples_is_identity_unverified() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, &complete_getprop("Pocket S", "fp-a"), "");
        executor.push_completed(0, "null\n", "");
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("serial-a".to_string()), executor);
        let mut guard = IdentityGuard::default();
        guard.configure(Some(&target("acme", "Pocket S", 33)));
        assert_eq!(
            guard.check(&mut runner, IdentityCheckPhase::PreOperation),
            Err(AdbCommandError::DeviceIdentityUnverified {
                phase: IdentityCheckPhase::PreOperation
            })
        );

        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(0, &complete_getprop("Pocket S", "fp-a"), "");
        executor.push_completed(0, "a1b2\n", "");
        executor.push_completed(0, &complete_getprop("Pocket X", "fp-a"), "");
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("serial-a".to_string()), executor);
        let mut guard = IdentityGuard::default();
        guard.configure(Some(&target("acme", "Pocket S", 33)));
        assert_eq!(
            guard.check(&mut runner, IdentityCheckPhase::PreOperation),
            Err(AdbCommandError::DeviceIdentityUnverified {
                phase: IdentityCheckPhase::PreOperation
            })
        );
    }

    #[test]
    fn reviewed_serial_binding_mismatch_blocks_before_identity_sampling() {
        let executor = FakeAdbCommandExecutor::default();
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("serial-a".to_string()), executor);
        let mut guard = IdentityGuard::default();
        let mut reviewed = target("acme", "Pocket S", 33);
        reviewed.serial = "serial-b".to_string();
        guard.configure(Some(&reviewed));

        assert_eq!(
            guard.check(&mut runner, IdentityCheckPhase::PreOperation),
            Err(AdbCommandError::DeviceIdentityChanged {
                phase: IdentityCheckPhase::PreOperation
            })
        );
        assert!(runner.executor().calls().is_empty());
    }

    #[test]
    fn insufficient_reviewed_target_fails_closed_before_any_identity_probe() {
        let mut cases = Vec::new();
        for value in [None, Some(""), Some("null"), Some("unknown")] {
            let mut reviewed = target("acme", "Pocket S", 33);
            reviewed.manufacturer = value.map(str::to_string);
            cases.push(("manufacturer", reviewed));
        }
        for value in [None, Some(""), Some("null"), Some("unknown")] {
            let mut reviewed = target("acme", "Pocket S", 33);
            reviewed.model = value.map(str::to_string);
            cases.push(("model", reviewed));
        }
        for value in [None, Some(0), Some(-1)] {
            let mut reviewed = target("acme", "Pocket S", 33);
            reviewed.android_api_level = value;
            cases.push(("android_api_level", reviewed));
        }
        let mut reviewed = target("acme", "Pocket S", 33);
        reviewed.android_api_level = Some(i64::from(u32::MAX) + 1);
        cases.push(("android_api_level_outside_u32", reviewed));
        for serial in ["", "   "] {
            let mut reviewed = target("acme", "Pocket S", 33);
            reviewed.serial = serial.to_string();
            cases.push(("serial", reviewed));
        }

        for (field, reviewed) in cases {
            let executor = FakeAdbCommandExecutor::default();
            let mut runner =
                AdbCommandRunner::with_executor("adb", Some("serial-a".to_string()), executor);
            let mut guard = IdentityGuard::default();
            guard.configure(Some(&reviewed));

            assert_eq!(
                guard.check(&mut runner, IdentityCheckPhase::PreOperation),
                Err(AdbCommandError::DeviceIdentityUnverified {
                    phase: IdentityCheckPhase::PreOperation,
                }),
                "{field} target evidence must fail closed"
            );
            assert!(
                runner.executor().calls().is_empty(),
                "{field} insufficiency must not probe the device"
            );
            assert!(
                runner.executor().operations().is_empty(),
                "{field} insufficiency must not issue an intended device operation"
            );
            assert!(
                guard.baseline.is_none(),
                "{field} insufficiency must not establish a baseline"
            );
        }
    }

    #[test]
    fn complete_reviewed_target_establishes_baseline_and_valid_mismatch_is_changed() {
        let mut executor = FakeAdbCommandExecutor::default();
        push_complete_sample(&mut executor, "a1b2");
        push_complete_sample(&mut executor, "a1b2");
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("serial-a".to_string()), executor);
        let mut guard = IdentityGuard::default();
        guard.configure(Some(&target("acme", "Pocket S", 33)));

        guard
            .check(&mut runner, IdentityCheckPhase::PreOperation)
            .expect("complete reviewed evidence should establish a baseline");
        assert!(guard.baseline.is_some());

        let mut executor = FakeAdbCommandExecutor::default();
        push_complete_sample(&mut executor, "a1b2");
        push_complete_sample(&mut executor, "a1b2");
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("serial-a".to_string()), executor);
        let mut guard = IdentityGuard::default();
        guard.configure(Some(&target("different", "Pocket S", 33)));

        assert_eq!(
            guard.check(&mut runner, IdentityCheckPhase::PreOperation),
            Err(AdbCommandError::DeviceIdentityChanged {
                phase: IdentityCheckPhase::PreOperation,
            })
        );
        assert_eq!(runner.executor().calls().len(), 4);
        assert!(guard.baseline.is_none());
    }

    #[test]
    fn identity_probe_preserves_existing_timeout_process_and_transport_precedence() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_timed_out();
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("serial-a".to_string()), executor);
        let mut guard = IdentityGuard::default();
        guard.configure(Some(&target("acme", "Pocket S", 33)));
        assert!(matches!(
            guard.check(&mut runner, IdentityCheckPhase::PreOperation),
            Err(AdbCommandError::TimedOut { .. })
        ));

        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_process_failed(ProcessFailureKind::StdoutOverflow);
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("serial-a".to_string()), executor);
        let mut guard = IdentityGuard::default();
        guard.configure(Some(&target("acme", "Pocket S", 33)));
        assert!(matches!(
            guard.check(&mut runner, IdentityCheckPhase::PreOperation),
            Err(AdbCommandError::ProcessFailed {
                kind: ProcessFailureKind::StdoutOverflow,
                ..
            })
        ));

        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(1, "", "error: device offline");
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("serial-a".to_string()), executor);
        let mut guard = IdentityGuard::default();
        guard.configure(Some(&target("acme", "Pocket S", 33)));
        assert_eq!(
            guard.check(&mut runner, IdentityCheckPhase::PreOperation),
            Err(AdbCommandError::DeviceOffline)
        );
    }

    #[test]
    fn guarded_operation_keeps_untrustworthy_failures_without_a_post_probe() {
        for failure in [
            FailureScript::Timeout,
            FailureScript::Process,
            FailureScript::Transport,
        ] {
            let mut executor = FakeAdbCommandExecutor::default();
            push_complete_sample(&mut executor, "a1b2");
            push_complete_sample(&mut executor, "a1b2");
            match failure {
                FailureScript::Timeout => executor.push_timed_out(),
                FailureScript::Process => {
                    executor.push_process_failed(ProcessFailureKind::StdoutOverflow)
                }
                FailureScript::Transport => executor.push_completed(1, "", "error: device offline"),
            }
            let mut device = crate::executor::adb::RealAdbDevice::with_executor(
                "adb",
                Some("serial-a"),
                executor,
            );
            device.configure_identity_guard(Some(&target("acme", "Pocket S", 33)));
            assert!(device
                .install_apk(std::path::Path::new("fixture.apk"), true)
                .is_err());
            assert_eq!(device.command_executor().operations().len(), 5);
            assert_eq!(
                device.command_executor().operations()[4],
                ProcessOperation::Install
            );
        }
    }

    #[test]
    fn completed_ordinary_failure_retains_original_after_a_successful_post_probe() {
        let mut executor = FakeAdbCommandExecutor::default();
        push_complete_sample(&mut executor, "a1b2");
        push_complete_sample(&mut executor, "a1b2");
        executor.push_completed(1, "", "ordinary failure");
        push_complete_sample(&mut executor, "a1b2");
        push_complete_sample(&mut executor, "a1b2");
        let mut device =
            crate::executor::adb::RealAdbDevice::with_executor("adb", Some("serial-a"), executor);
        device.configure_identity_guard(Some(&target("acme", "Pocket S", 33)));

        let error = device
            .install_apk(std::path::Path::new("fixture.apk"), true)
            .unwrap_err();

        assert_eq!(error, "The ADB command failed.");
        assert_eq!(device.command_executor().operations().len(), 9);
    }

    #[test]
    fn unknown_completed_probe_failure_becomes_identity_unverified() {
        let mut executor = FakeAdbCommandExecutor::default();
        executor.push_completed(1, "", "unexpected probe failure");
        let mut runner =
            AdbCommandRunner::with_executor("adb", Some("serial-a".to_string()), executor);
        let mut guard = IdentityGuard::default();
        guard.configure(Some(&target("acme", "Pocket S", 33)));
        assert_eq!(
            guard.check(&mut runner, IdentityCheckPhase::PreOperation),
            Err(AdbCommandError::DeviceIdentityUnverified {
                phase: IdentityCheckPhase::PreOperation
            })
        );
    }

    #[test]
    fn same_serial_changed_android_id_after_operation_stops_without_a_second_command() {
        let mut executor = FakeAdbCommandExecutor::default();
        push_complete_sample(&mut executor, "a1b2");
        push_complete_sample(&mut executor, "a1b2");
        push_complete_sample(&mut executor, "c3d4");
        push_complete_sample(&mut executor, "c3d4");
        let mut device =
            crate::executor::adb::RealAdbDevice::with_executor("adb", Some("serial-a"), executor);
        device.configure_identity_guard(Some(&target("acme", "Pocket S", 33)));
        assert!(device
            .install_apk(std::path::Path::new("fixture.apk"), true)
            .is_err());
        assert!(device
            .command_executor()
            .operations()
            .contains(&ProcessOperation::Install));
    }

    #[test]
    fn same_serial_changed_android_id_before_operation_blocks_mutation() {
        let mut executor = FakeAdbCommandExecutor::default();
        push_complete_sample(&mut executor, "a1b2");
        push_complete_sample(&mut executor, "a1b2");
        executor.push_completed(0, "", "");
        push_complete_sample(&mut executor, "a1b2");
        push_complete_sample(&mut executor, "a1b2");
        push_complete_sample(&mut executor, "c3d4");
        push_complete_sample(&mut executor, "c3d4");
        let mut device =
            crate::executor::adb::RealAdbDevice::with_executor("adb", Some("serial-a"), executor);
        device.configure_identity_guard(Some(&target("acme", "Pocket S", 33)));
        device
            .install_apk(std::path::Path::new("fixture.apk"), true)
            .unwrap();
        assert!(device
            .install_apk(std::path::Path::new("fixture.apk"), true)
            .is_err());
        assert_eq!(
            device
                .command_executor()
                .operations()
                .iter()
                .filter(|operation| **operation == ProcessOperation::Install)
                .count(),
            1
        );
    }

    fn target(manufacturer: &str, model: &str, android_api_level: i64) -> TargetDeviceBinding {
        TargetDeviceBinding {
            serial: "serial-a".to_string(),
            manufacturer: Some(manufacturer.to_string()),
            model: Some(model.to_string()),
            android_api_level: Some(android_api_level),
        }
    }

    fn push_complete_sample(executor: &mut FakeAdbCommandExecutor, android_id: &str) {
        executor.push_completed(0, &complete_getprop("Pocket S", "fp-a"), "");
        executor.push_completed(0, &format!("{android_id}\n"), "");
    }

    fn complete_getprop(model: &str, fingerprint: &str) -> String {
        format!(
            "[ro.product.manufacturer]: [Acme]\n\
[ro.product.brand]: [Acme]\n\
[ro.product.model]: [{model}]\n\
[ro.product.name]: [pocket]\n\
[ro.product.device]: [pocket]\n\
[ro.product.board]: [board]\n\
[ro.hardware]: [hardware]\n\
[ro.build.version.sdk]: [33]\n\
[ro.product.cpu.abilist]: [arm64-v8a, armeabi-v7a]\n\
[ro.build.fingerprint]: [{fingerprint}]\n"
        )
    }
}
