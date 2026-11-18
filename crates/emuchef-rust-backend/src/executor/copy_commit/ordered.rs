//! Reviewed ordering for CopyCommit operations with overlapping destinations.
//!
//! Each later snapshot records the projected state and deletion scope produced
//! by earlier copy operations. Execution accepts that state only after verifying
//! every predecessor report and the current destination manifest.

use std::path::Path;

use crate::executor::{inspect_android_setup_destination, ExecutorDevice, SandboxRoots};
use crate::planner::{ExecutionPlan, ExecutionStep};

use super::super::{
    active_copy_writer_scope, active_removable_volume_roots, canonical_active_writer_scope,
    copy_commit_report_failure, copy_writer_effects, paths_overlap,
    setup_destination_precondition_missing_paths, ExecutionState, StepFailure, StepRuntimeStatus,
};
use super::{
    manifest_matches_projection, CopyCommitDestinationKind, CopyCommitOutcome, CopyCommitReport,
    CopyCommitRequest, CopyEntryKind, CopyManifestEntry, ExecutionCopyCommitSnapshot,
};

pub(in crate::executor) fn current_copy_destination_manifest<D: ExecutorDevice>(
    device: &mut D,
    snapshot: &ExecutionCopyCommitSnapshot,
    source_path: &Path,
    sandbox: &SandboxRoots,
) -> Result<Vec<CopyManifestEntry>, String> {
    if device.uses_fake_copy_commit_adapter() {
        let request = match snapshot.destination_kind {
            CopyCommitDestinationKind::AppTarget => CopyCommitRequest::reviewed_directory(
                source_path,
                &sandbox.fake_device_root,
                &snapshot.destination_identity,
                snapshot.operation,
            ),
            CopyCommitDestinationKind::SharedDevicePath => {
                CopyCommitRequest::reviewed_shared_directory(
                    source_path,
                    &sandbox.fake_device_root,
                    &snapshot.destination_identity,
                    snapshot.operation,
                )
            }
            CopyCommitDestinationKind::SetupLocation => {
                let binding = snapshot
                    .setup_location_binding
                    .clone()
                    .ok_or_else(|| "Setup Location snapshot is missing its binding".to_string())?;
                let request = CopyCommitRequest::reviewed_fake_setup_directory(
                    source_path,
                    &sandbox.fake_device_root,
                    binding,
                    snapshot.operation,
                )
                .map_err(|error| error.to_string())?;
                if request.setup_destination_precondition != snapshot.setup_destination_precondition
                {
                    return Err(
                        "Setup Location destination precondition changed after review".to_string(),
                    );
                }
                return Ok(request.destination_manifest);
            }
        }
        .map_err(|error| error.to_string())?;
        return Ok(request.destination_manifest);
    }

    match snapshot.destination_kind {
        CopyCommitDestinationKind::SetupLocation => {
            let binding = snapshot
                .setup_location_binding
                .as_ref()
                .ok_or_else(|| "Setup Location snapshot is missing its binding".to_string())?;
            let (precondition, manifest) = inspect_android_setup_destination(device, binding)?;
            if Some(&precondition) != snapshot.setup_destination_precondition.as_ref() {
                return Err(
                    "Setup Location destination precondition changed after review".to_string(),
                );
            }
            Ok(manifest)
        }
        CopyCommitDestinationKind::AppTarget | CopyCommitDestinationKind::SharedDevicePath => {
            let current_binding = device
                .copy_commit_path_binding(&snapshot.destination_identity)
                .map_err(|error| error.message)?;
            if snapshot.destination_binding.as_deref() != Some(current_binding.as_str()) {
                return Err("selected destination identity changed after review".to_string());
            }
            device
                .copy_commit_tree_manifest(&snapshot.destination_identity)
                .map_err(|error| error.message)
        }
    }
}

pub(in crate::executor) fn reviewed_destination_manifest_for_execution(
    plan: &ExecutionPlan,
    state: &ExecutionState,
    snapshot: &ExecutionCopyCommitSnapshot,
    current_manifest: Vec<CopyManifestEntry>,
    simulation: bool,
) -> Result<Vec<CopyManifestEntry>, String> {
    let expected_manifest = ordered_runtime_destination_state(plan, state, snapshot)?;
    let seed_simulation_baseline = simulation
        && snapshot.ordered_predecessor_step_ids.is_empty()
        && current_manifest.is_empty();
    if current_manifest != expected_manifest && !seed_simulation_baseline {
        return Err(
            "selected destination changed outside the reviewed ordered copy sequence; prepare a fresh review"
                .to_string(),
        );
    }
    if seed_simulation_baseline {
        Ok(expected_manifest)
    } else {
        Ok(current_manifest)
    }
}

pub(in crate::executor) fn prepare_ordered_copy_projections(
    steps: &[ExecutionStep],
    snapshots: &mut [ExecutionCopyCommitSnapshot],
) -> Result<(), String> {
    let recipe_by_step_id = steps
        .iter()
        .map(|step| (step.id.as_str(), step.recipe_ref.as_str()))
        .collect::<std::collections::HashMap<_, _>>();
    let removable_volume_roots = active_removable_volume_roots(snapshots)?;
    let scopes = snapshots
        .iter()
        .map(|snapshot| {
            let scope = active_copy_writer_scope(snapshot)?;
            canonical_active_writer_scope(scope, &removable_volume_roots)
        })
        .collect::<Result<Vec<_>, String>>()?;

    for current_index in 0..snapshots.len() {
        let current_scope = &scopes[current_index];
        let current = &snapshots[current_index];
        let current_recipe = recipe_by_step_id
            .get(current.step_id.as_str())
            .ok_or_else(|| format!("copy snapshot {} has no execution step", current.step_id))?;
        let mut projected_before = snapshots[current_index]
            .destination_initial_manifest
            .clone();
        let mut predecessor_step_ids = Vec::new();
        for predecessor_index in 0..current_index {
            let predecessor_scope = &scopes[predecessor_index];
            if !destination_scopes_overlap(current_scope, predecessor_scope) {
                continue;
            }
            let predecessor = &snapshots[predecessor_index];
            let predecessor_recipe = recipe_by_step_id
                .get(predecessor.step_id.as_str())
                .ok_or_else(|| {
                    format!(
                        "copy snapshot {} has no execution step",
                        predecessor.step_id
                    )
                })?;
            if current_recipe == predecessor_recipe {
                return Err(format!(
                    "copy steps {} and {} from the same Recipe have overlapping destinations",
                    predecessor.step_id, current.step_id
                ));
            }
            if !predecessor.has_ordered_projection() {
                return Err(format!(
                    "ordered copy step {} has no reviewed destination projection",
                    predecessor.step_id
                ));
            }
            projected_before = project_manifest_between_scopes(
                projected_before,
                current_scope,
                predecessor_scope,
                &predecessor.projected_destination_after_manifest,
            )?;
            predecessor_step_ids.push(predecessor.step_id.clone());
        }
        snapshots[current_index]
            .set_ordered_projection(projected_before, predecessor_step_ids)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(in crate::executor) fn validate_ordered_writer_preconditions(
    snapshots: &[ExecutionCopyCommitSnapshot],
) -> Result<(), String> {
    if !snapshots
        .iter()
        .any(|snapshot| snapshot.destination_kind == CopyCommitDestinationKind::SetupLocation)
    {
        return Ok(());
    }
    let removable_volume_roots = active_removable_volume_roots(snapshots)?;
    let active = snapshots
        .iter()
        .map(|snapshot| {
            let scope = active_copy_writer_scope(snapshot)?;
            let scope = canonical_active_writer_scope(scope, &removable_volume_roots)?;
            let (effects, creation_paths) = copy_writer_effects(snapshot, &scope)?;
            let missing_precondition_paths =
                setup_destination_precondition_missing_paths(snapshot, &scope)?;
            Ok((
                snapshot,
                effects,
                creation_paths,
                missing_precondition_paths,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;

    for (index, (earlier, _, earlier_creations, _)) in active.iter().enumerate() {
        for (later, _, _, later_missing) in active.iter().skip(index + 1) {
            if later_missing.is_empty() {
                continue;
            }
            if earlier_creations.iter().any(|created| {
                later_missing
                    .iter()
                    .any(|missing| paths_overlap(created, missing))
            }) {
                return Err(format!(
                    "ordered copy step {} creates a path that later Setup Location step {} reviewed as absent; prepare a fresh review",
                    earlier.step_id, later.step_id
                ));
            }
        }
    }
    Ok(())
}

fn destination_scopes_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn project_manifest_between_scopes(
    mut current_manifest: Vec<CopyManifestEntry>,
    current_scope: &Path,
    predecessor_scope: &Path,
    predecessor_after_manifest: &[CopyManifestEntry],
) -> Result<Vec<CopyManifestEntry>, String> {
    if current_scope == predecessor_scope {
        return Ok(predecessor_after_manifest.to_vec());
    }

    if predecessor_scope.starts_with(current_scope) {
        let relative = predecessor_scope
            .strip_prefix(current_scope)
            .map_err(|_| "ordered copy destination scope is invalid".to_string())?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        let prefix = format!("{relative}/");
        let root = current_manifest
            .iter()
            .find(|entry| entry.relative_path == relative)
            .ok_or_else(|| {
                "ordered copy destination is no longer present at its reviewed parent scope; prepare a fresh review".to_string()
            })?;
        if root.kind != CopyEntryKind::Directory {
            return Err(
                "ordered copy destination is covered by a file after an earlier operation; prepare a fresh review".to_string(),
            );
        }
        current_manifest.retain(|entry| !entry.relative_path.starts_with(&prefix));
        current_manifest.extend(predecessor_after_manifest.iter().cloned().map(|mut entry| {
            entry.relative_path = format!("{relative}/{}", entry.relative_path);
            entry
        }));
    } else if current_scope.starts_with(predecessor_scope) {
        let relative = current_scope
            .strip_prefix(predecessor_scope)
            .map_err(|_| "ordered copy destination scope is invalid".to_string())?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        let root = current_manifest
            .iter()
            .find(|entry| entry.relative_path == relative)
            .ok_or_else(|| {
                "the reviewed destination parent does not contain the nested copy scope; prepare a fresh review".to_string()
            })?;
        if root.kind != CopyEntryKind::Directory {
            return Err(
                "the reviewed destination parent contains a file at the nested copy scope; prepare a fresh review".to_string(),
            );
        }
        let prefix = format!("{relative}/");
        current_manifest = predecessor_after_manifest
            .iter()
            .filter(|entry| entry.relative_path.starts_with(&prefix))
            .cloned()
            .map(|mut entry| {
                entry.relative_path = entry.relative_path[prefix.len()..].to_string();
                entry
            })
            .collect();
    }
    current_manifest.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(current_manifest)
}

pub(in crate::executor) fn validate_ordered_copy_snapshot_links(
    plan: &ExecutionPlan,
) -> Result<(), String> {
    let removable_volume_roots = active_removable_volume_roots(&plan.copy_commit_snapshots)?;
    let steps_by_id = plan
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| (step.id.as_str(), (index, step)))
        .collect::<std::collections::HashMap<_, _>>();
    let mut ordered = plan
        .copy_commit_snapshots
        .iter()
        .map(|snapshot| {
            let (index, step) = steps_by_id
                .get(snapshot.step_id.as_str())
                .copied()
                .ok_or_else(|| {
                    format!("copy snapshot {} has no execution step", snapshot.step_id)
                })?;
            let scope = canonical_active_writer_scope(
                active_copy_writer_scope(snapshot)?,
                &removable_volume_roots,
            )?;
            Ok((index, snapshot, scope, step.recipe_ref.as_str()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    ordered.sort_by_key(|(index, _, _, _)| *index);

    for current_index in 0..ordered.len() {
        let (_, current, current_scope, current_recipe) = &ordered[current_index];
        let overlapping_predecessors = ordered[..current_index]
            .iter()
            .filter(|(_, _, prior_scope, _)| {
                destination_scopes_overlap(current_scope.as_path(), prior_scope.as_path())
            })
            .collect::<Vec<_>>();
        if overlapping_predecessors
            .iter()
            .any(|(_, _, _, prior_recipe)| prior_recipe == current_recipe)
        {
            return Err(format!(
                "copy step {} overlaps another copy from the same Recipe",
                current.step_id
            ));
        }
        let expected_predecessors = overlapping_predecessors
            .iter()
            .map(|(_, prior, _, _)| prior.step_id.as_str())
            .collect::<Vec<_>>();
        let actual_predecessors = current
            .ordered_predecessor_step_ids
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if expected_predecessors != actual_predecessors {
            return Err(format!(
                "copy step {} has incomplete or reordered destination predecessors",
                current.step_id
            ));
        }
        if !expected_predecessors.is_empty() && !current.has_ordered_projection() {
            return Err(format!(
                "copy step {} overlaps an earlier writer but has no ordered review projection",
                current.step_id
            ));
        }
        if current.has_ordered_projection() {
            let mut projected_before = current.destination_initial_manifest.clone();
            for (_, predecessor, predecessor_scope, _) in overlapping_predecessors {
                if !predecessor.has_ordered_projection() {
                    return Err(format!(
                        "copy predecessor {} has no ordered result projection",
                        predecessor.step_id
                    ));
                }
                projected_before = project_manifest_between_scopes(
                    projected_before,
                    current_scope.as_path(),
                    predecessor_scope.as_path(),
                    &predecessor.projected_destination_after_manifest,
                )?;
            }
            if !manifest_matches_projection(&current.destination_before_manifest, &projected_before)
            {
                return Err(format!(
                    "copy step {} destination-before state does not match its ordered predecessor projections",
                    current.step_id
                ));
            }
        }
    }
    Ok(())
}

pub(in crate::executor) fn copy_commit_report_projection_failure(
    report: super::CopyCommitReport,
    reason: &str,
) -> StepFailure {
    let mut failure = copy_commit_report_failure(report);
    failure.message = format!(
        "{reason}; the destination result is retained and later overlapping copies require a fresh review"
    );
    failure
}

pub(in crate::executor) fn validate_ordered_copy_commit_result(
    plan: &ExecutionPlan,
    step_id: &str,
    request: &CopyCommitRequest,
    report: CopyCommitReport,
) -> Result<CopyCommitReport, StepFailure> {
    let Some(snapshot) = plan
        .copy_commit_snapshots
        .iter()
        .find(|snapshot| snapshot.step_id == step_id)
    else {
        return Err(copy_commit_report_projection_failure(
            report,
            "reviewed copy snapshot disappeared during execution",
        ));
    };
    if !snapshot.has_ordered_projection() {
        return Ok(report);
    }
    let report_matches = report.succeeded
        && matches!(
            report.outcome,
            CopyCommitOutcome::Succeeded | CopyCommitOutcome::NoChange
        )
        && report.before_manifest == request.destination_manifest
        && report.reviewed_deletion_set == snapshot.reviewed_deletion_set
        && report.after_manifest.as_ref().is_some_and(|after| {
            manifest_matches_projection(&snapshot.projected_destination_after_manifest, after)
        });
    if !report_matches {
        return Err(copy_commit_report_projection_failure(
            report,
            "copy result does not match its reviewed ordered destination projection",
        ));
    }
    Ok(report)
}

pub(in crate::executor) fn ordered_runtime_destination_state(
    plan: &ExecutionPlan,
    state: &ExecutionState,
    current: &ExecutionCopyCommitSnapshot,
) -> Result<Vec<CopyManifestEntry>, String> {
    if !current.has_ordered_projection() && current.ordered_predecessor_step_ids.is_empty() {
        return Ok(current.destination_before_manifest.clone());
    }
    current
        .validate_ordered_projection()
        .map_err(|error| error.to_string())?;
    if current.ordered_predecessor_step_ids.is_empty() {
        return Ok(current.destination_initial_manifest.clone());
    }
    if current.destination_initial_digest_sha256.is_none() {
        return Err(format!(
            "ordered copy step {} has no initial destination snapshot; prepare a fresh review",
            current.step_id
        ));
    }

    let removable_volume_roots = active_removable_volume_roots(&plan.copy_commit_snapshots)?;
    let current_scope =
        canonical_active_writer_scope(active_copy_writer_scope(current)?, &removable_volume_roots)?;
    let mut expected = current.destination_initial_manifest.clone();
    for predecessor_id in &current.ordered_predecessor_step_ids {
        let predecessor = plan
            .copy_commit_snapshots
            .iter()
            .find(|snapshot| snapshot.step_id == *predecessor_id)
            .ok_or_else(|| {
                format!(
                    "ordered copy predecessor {predecessor_id} is missing; prepare a fresh review"
                )
            })?;
        match state.steps.get(predecessor_id).map(|step| &step.status) {
            Some(StepRuntimeStatus::Succeeded) => {}
            Some(status) => {
                return Err(format!(
                    "ordered copy step {} is blocked because predecessor {} finished as {status:?}",
                    current.step_id, predecessor_id
                ));
            }
            None => {
                return Err(format!(
                    "ordered copy step {} is blocked because predecessor {} has no run result",
                    current.step_id, predecessor_id
                ));
            }
        }
        predecessor
            .validate_ordered_projection()
            .map_err(|error| error.to_string())?;
        let report = state
            .steps
            .get(predecessor_id)
            .and_then(|step| step.outputs.get("copy_commit_report"))
            .map(|output| output.value.clone())
            .ok_or_else(|| {
                format!("ordered copy predecessor {predecessor_id} has no verified commit report")
            })?;
        let report: CopyCommitReport = serde_json::from_value(report).map_err(|error| {
            format!(
                "ordered copy predecessor {predecessor_id} has an invalid commit report: {error}"
            )
        })?;
        if !report.succeeded
            || !matches!(
                report.outcome,
                super::CopyCommitOutcome::Succeeded | super::CopyCommitOutcome::NoChange
            )
        {
            return Err(format!(
                "ordered copy predecessor {predecessor_id} did not report a complete result"
            ));
        }
        let after_manifest = report.after_manifest.ok_or_else(|| {
            format!("ordered copy predecessor {predecessor_id} has no after manifest")
        })?;
        if !manifest_matches_projection(
            &predecessor.projected_destination_after_manifest,
            &after_manifest,
        ) {
            return Err(format!(
                "ordered copy predecessor {predecessor_id} result differs from its reviewed projection"
            ));
        }
        let predecessor_scope = canonical_active_writer_scope(
            active_copy_writer_scope(predecessor)?,
            &removable_volume_roots,
        )?;
        expected = project_manifest_between_scopes(
            expected,
            &current_scope,
            &predecessor_scope,
            &after_manifest,
        )?;
    }
    if !manifest_matches_projection(&current.destination_before_manifest, &expected) {
        return Err(format!(
            "ordered copy step {} predecessor results do not match its reviewed destination projection",
            current.step_id
        ));
    }
    Ok(expected)
}
