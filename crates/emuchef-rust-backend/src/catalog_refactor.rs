//! Typed App identity refactor planning.
//!
//! This module rewrites only schema-defined App references. It deliberately
//! does not inspect arbitrary strings or metadata, so a catalog refactor cannot
//! become a generic text replacement.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app_definition_document::CatalogIdentity;
use crate::app_definition_persistence::FileIdentity;
use crate::authored_models::{emit_app_definition_yaml, is_valid_identifier, AppDefinitionV1};
use crate::catalog_change::{
    CatalogChangeCoordinator, CatalogRefactorRecovery, CatalogRefactorRecoveryIntent,
    DurableSourceDeletion, FileSnapshot, OperationPhase, OperationRecord, PublicationGuard,
};
use crate::model::{ParamValue, Recipe, RecipeArtifact};
use crate::yaml;

static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);

fn catalog_change_operation_id(timestamp_nanos: u128, process_id: u32, sequence: u64) -> String {
    format!("catalog-change-{timestamp_nanos:x}-{process_id:x}-{sequence:x}")
}

/// One explicit catalog identity operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CatalogChangeIntent {
    RenameApp {
        app_id: String,
        new_id: String,
    },
    RenameArtifact {
        app_id: String,
        artifact_id: String,
        new_id: String,
    },
    RenameTarget {
        app_id: String,
        target_id: String,
        new_id: String,
    },
    DeleteApp {
        app_id: String,
    },
}

/// Immutable editor-session facts captured when a preview is requested.
///
/// The fingerprint covers the persisted App identity, the draft YAML, and the
/// exact opened file revision. Apply must reject the preview if any of those
/// facts change before confirmation, even though catalog disk freshness is
/// checked separately at the publication boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CatalogPreviewBinding {
    pub(crate) document_id: String,
    pub(crate) fingerprint: String,
}

impl CatalogPreviewBinding {
    pub(crate) fn for_app_document(
        document_id: &str,
        identity: &CatalogIdentity,
        current_yaml: &str,
        opened_bytes: &[u8],
        opened_identity: &FileIdentity,
    ) -> Self {
        let mut digest = Sha256::new();
        for field in [
            document_id.as_bytes(),
            identity.id.as_bytes(),
            identity.package_id.as_bytes(),
            identity.path.to_string_lossy().as_bytes(),
            identity.authored_root.to_string_lossy().as_bytes(),
            current_yaml.as_bytes(),
            opened_bytes,
        ] {
            digest.update((field.len() as u64).to_be_bytes());
            digest.update(field);
        }
        let identity_bytes = format!("{opened_identity:?}");
        digest.update((identity_bytes.len() as u64).to_be_bytes());
        digest.update(identity_bytes.as_bytes());
        Self {
            document_id: document_id.to_owned(),
            fingerprint: hex(&digest.finalize()),
        }
    }
}

/// Exact typed location affected by a catalog operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceSite {
    pub path: PathBuf,
    pub object: String,
    pub field: String,
}

/// One open editor session whose unsaved state prevents publication.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogChangeBlocker {
    pub path: PathBuf,
    pub reason: String,
}

/// Sanitized, authoritative impact preview returned before confirmation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogChangePreview {
    pub operation_id: String,
    pub preview_token: String,
    pub intent: CatalogChangeIntent,
    pub references: Vec<ReferenceSite>,
    pub changed_files: Vec<PathBuf>,
    pub blockers: Vec<CatalogChangeBlocker>,
    pub snapshot_digest: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedFile {
    pub path: PathBuf,
    pub original_snapshot: Option<FileSnapshot>,
    pub permission_source: Option<FileSnapshot>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedCatalogChange {
    pub preview: CatalogChangePreview,
    pub authored_root: PathBuf,
    pub files: Vec<PreparedFile>,
    pub delete_source: Option<(PathBuf, FileSnapshot)>,
    pub observed_files: Vec<(PathBuf, FileSnapshot)>,
    pub inventory_paths: Vec<PathBuf>,
    pub binding: Option<CatalogPreviewBinding>,
}

/// Process-local preview handles. Durable apply status is owned by the
/// catalog-change journal, while preview tokens are deliberately single-use.
#[derive(Debug, Default)]
pub struct CatalogRefactorSession {
    previews: HashMap<String, PreparedCatalogChange>,
    operation_roots: HashMap<String, PathBuf>,
    storage_base: Option<PathBuf>,
}

/// Durable client-visible operation state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogChangeState {
    Rejected,
    Completed,
    RecoveryRequired,
    InProgress,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogChangeStatus {
    pub operation_id: String,
    pub state: CatalogChangeState,
    pub changed_files: Vec<PathBuf>,
}

impl CatalogRefactorSession {
    pub(crate) fn storage_base(&self) -> Option<&Path> {
        self.storage_base.as_deref()
    }

    /// Capture the exact authoritative catalog snapshot and return a single-use
    /// confirmation token bound to that snapshot.
    pub fn preview(
        &mut self,
        authored_root: impl AsRef<Path>,
        intent: CatalogChangeIntent,
    ) -> Result<CatalogChangePreview, String> {
        self.preview_with_binding(authored_root, intent, None)
    }

    pub(crate) fn preview_for_document(
        &mut self,
        authored_root: impl AsRef<Path>,
        intent: CatalogChangeIntent,
        binding: CatalogPreviewBinding,
    ) -> Result<CatalogChangePreview, String> {
        self.preview_with_binding(authored_root, intent, Some(binding))
    }

    fn preview_with_binding(
        &mut self,
        authored_root: impl AsRef<Path>,
        intent: CatalogChangeIntent,
        binding: Option<CatalogPreviewBinding>,
    ) -> Result<CatalogChangePreview, String> {
        let root = fs::canonicalize(authored_root.as_ref()).map_err(|error| error.to_string())?;
        let coordinator = self.coordinator(&root)?;
        let mut prepared = prepare_catalog_change(&coordinator, &root, intent)?;
        prepared.binding = binding;
        let preview = prepared.preview.clone();
        self.operation_roots
            .insert(preview.operation_id.clone(), prepared.authored_root.clone());
        self.previews
            .insert(preview.preview_token.clone(), prepared);
        Ok(preview)
    }

    pub(crate) fn preview_binding(&self, operation_id: &str) -> Option<CatalogPreviewBinding> {
        self.previews
            .values()
            .find(|prepared| prepared.preview.operation_id == operation_id)
            .and_then(|prepared| prepared.binding.clone())
    }

    pub(crate) fn authored_root_for_operation(&self, operation_id: &str) -> Option<PathBuf> {
        self.operation_roots.get(operation_id).cloned().or_else(|| {
            self.previews
                .values()
                .find(|prepared| prepared.preview.operation_id == operation_id)
                .map(|prepared| prepared.authored_root.clone())
        })
    }

    /// Consume a preview token. It cannot authorize a second publication even
    /// when the first response is lost.
    pub(crate) fn take(
        &mut self,
        operation_id: &str,
        preview_token: &str,
    ) -> Result<PreparedCatalogChange, String> {
        let prepared = self.previews.get(preview_token).ok_or_else(|| {
            "The catalog-change preview token is unknown or already used.".to_string()
        })?;
        if prepared.preview.operation_id != operation_id {
            return Err("The preview token belongs to a different operation.".into());
        }
        self.previews
            .remove(preview_token)
            .ok_or_else(|| "The catalog-change preview token was already used.".to_string())
    }

    /// Detect references in a dirty in-memory Recipe that are not yet present
    /// in its on-disk file. Disk impact discovery cannot see this unsaved
    /// intent, so the owning session manager uses this check before apply.
    pub(crate) fn dirty_recipe_references_identity(
        &self,
        operation_id: &str,
        authored_root: Option<&Path>,
        recipe_path: &Path,
        recipe: &Recipe,
    ) -> Result<bool, String> {
        let Some(prepared) = self
            .previews
            .values()
            .find(|prepared| prepared.preview.operation_id == operation_id)
        else {
            return Ok(false);
        };
        let Some(authored_root) = authored_root else {
            return Ok(false);
        };
        let root = fs::canonicalize(authored_root).map_err(|error| {
            format!("The dirty Recipe authored root could not be verified: {error}")
        })?;
        if root != prepared.authored_root {
            return Ok(false);
        }

        let references = match &prepared.preview.intent {
            CatalogChangeIntent::DeleteApp { app_id } => {
                discover_app_references(recipe, recipe_path, app_id)
            }
            intent => {
                let mut candidate = recipe.clone();
                rewrite_recipe(&mut candidate, recipe_path.to_path_buf(), intent)
            }
        };
        Ok(!references.is_empty())
    }

    /// Consume and apply a preview under the shared publication lock.
    ///
    /// Repeating a completed operation returns its durable terminal status
    /// before token validation, so a lost apply response cannot republish.
    pub fn apply(
        &mut self,
        operation_id: &str,
        preview_token: &str,
        dirty_paths: &[PathBuf],
        dirty_reference_paths: &[PathBuf],
        reconcile: impl FnOnce(&[PathBuf]) -> Result<(), String>,
    ) -> Result<CatalogChangeStatus, String> {
        if let Some(record) = self.find_durable_operation(operation_id)? {
            let changed_files = status_changed_files(Path::new(""), &record);
            if record.phase.is_terminal()
                || self.authored_root_for_operation(operation_id).is_none()
            {
                return Ok(status_from_record_with_activity(
                    record,
                    changed_files,
                    false,
                ));
            }
        }
        let root = self
            .operation_roots
            .get(operation_id)
            .cloned()
            .ok_or_else(|| {
                "The catalog-change operation is unknown in this session.".to_string()
            })?;
        let coordinator = self.coordinator(&root)?;
        let (record, publication_active) = coordinator
            .operation_for_status(operation_id)
            .map_err(|error| error.to_string())?;
        if let Some(record) = record {
            if record.phase.is_terminal() {
                let changed_files = status_changed_files(&root, &record);
                return Ok(status_from_record(record, changed_files));
            }
            let changed_files = status_changed_files(&root, &record);
            return Ok(status_from_record_with_activity(
                record,
                changed_files,
                publication_active,
            ));
        }
        let prepared = self.take(operation_id, preview_token)?;
        let guard = coordinator
            .acquire_publication()
            .map_err(|error| error.to_string())?;
        if let Some(record) = coordinator
            .operation(operation_id)
            .map_err(|error| error.to_string())?
        {
            let changed_files = status_changed_files(&root, &record);
            return Ok(status_from_record(record, changed_files));
        }
        let mut parent = OperationRecord::new(
            operation_id,
            "catalog-refactor",
            &prepared.preview.snapshot_digest,
        );
        parent.snapshot_digest = Some(prepared.preview.snapshot_digest.clone());
        parent.changed_files = prepared.preview.changed_files.clone();
        let mut dependent_files = prepared
            .preview
            .references
            .iter()
            .map(|reference| reference.path.clone())
            .collect::<Vec<_>>();
        dependent_files.sort();
        dependent_files.dedup();
        parent.catalog_refactor_recovery = Some(CatalogRefactorRecovery {
            intent: durable_recovery_intent(&prepared.preview.intent),
            dependent_files,
        });
        parent.delete_source =
            prepared
                .delete_source
                .as_ref()
                .map(|(path, snapshot)| DurableSourceDeletion {
                    source: path
                        .strip_prefix(&prepared.authored_root)
                        .expect("prepared source is inside authored root")
                        .to_path_buf(),
                    expected_digest: digest(&snapshot.bytes),
                    expected_identity: Some(snapshot.identity.clone()),
                });
        parent.child_operation_ids = prepared
            .files
            .iter()
            .map(|file| {
                coordinator
                    .child_write_operation_id(operation_id, &file.path)
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let child_operation_ids = parent.child_operation_ids.clone();
        coordinator
            .begin_operation(&guard, parent)
            .map_err(|error| error.to_string())?;

        let affected = prepared_changed_paths(&prepared);
        let mut blocking_dirty = dirty_paths
            .iter()
            .filter(|path| {
                let normalized = fs::canonicalize(path).unwrap_or_else(|_| (*path).clone());
                affected
                    .iter()
                    .any(|affected| affected.as_path() == normalized.as_path())
            })
            .chain(dirty_reference_paths.iter())
            .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
            .collect::<Vec<_>>();
        blocking_dirty.sort();
        blocking_dirty.dedup();
        if !blocking_dirty.is_empty() {
            coordinator
                .transition(&guard, operation_id, OperationPhase::Rejected)
                .map_err(|error| error.to_string())?;
            return Err(format!(
                "Unsaved affected documents block the catalog change: {}",
                blocking_dirty
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Err(message) = verify_prepared_snapshot(&coordinator, &prepared) {
            coordinator
                .transition(&guard, operation_id, OperationPhase::Rejected)
                .map_err(|error| error.to_string())?;
            return Err(message);
        }

        for file in &prepared.files {
            if let Err(error) = coordinator.stage_file_write_for_parent_with_permissions(
                &guard,
                operation_id,
                &file.path,
                file.original_snapshot.as_ref(),
                &file.bytes,
                file.permission_source.as_ref(),
            ) {
                coordinator
                    .reject_group_before_publication(&guard, operation_id)
                    .map_err(|rejection| rejection.to_string())?;
                return Err(error.to_string());
            }
        }
        if let Err(message) =
            validate_staged_candidates(&coordinator, &guard, &child_operation_ids, &prepared)
        {
            coordinator
                .reject_group_before_publication(&guard, operation_id)
                .map_err(|error| error.to_string())?;
            return Err(message);
        }
        if let Err(message) = verify_prepared_snapshot(&coordinator, &prepared) {
            coordinator
                .reject_group_before_publication(&guard, operation_id)
                .map_err(|error| error.to_string())?;
            return Err(message);
        }
        coordinator
            .transition(&guard, operation_id, OperationPhase::Staged)
            .map_err(|error| error.to_string())?;
        coordinator
            .transition(&guard, operation_id, OperationPhase::Publishing)
            .map_err(|error| error.to_string())?;
        let mut published_children: Vec<String> = Vec::new();
        for child in &child_operation_ids {
            if let Err(error) =
                coordinator.publish_staged_file_write_until_reconciled(&guard, child)
            {
                let recovery = rollback_group_publication(
                    &coordinator,
                    &guard,
                    operation_id,
                    &published_children,
                )
                .err();
                return Err(recovery.map_or_else(
                    || error.to_string(),
                    |recovery| format!("{error}; {recovery}"),
                ));
            }
            published_children.push(child.clone());
        }
        if let Some((source, expected)) = &prepared.delete_source {
            let current = coordinator
                .snapshot_file(source)
                .map_err(|error| error.to_string())?;
            if current.as_ref() != Some(expected) {
                let recovery = rollback_group_publication(
                    &coordinator,
                    &guard,
                    operation_id,
                    &published_children,
                )
                .err();
                let message =
                    "The source App Definition changed before its ownership-safe removal.";
                return Err(recovery.map_or_else(
                    || message.into(),
                    |recovery| format!("{message}; {recovery}"),
                ));
            }
            if let Err(error) = fs::remove_file(source) {
                ensure_recovery_required(&coordinator, &guard, operation_id)
                    .map_err(|recovery| format!("{error}; {recovery}"))?;
                return Err(format!(
                    "The source App Definition could not be removed; recovery is required: {error}"
                ));
            }
            if let Some(parent) = source.parent() {
                if let Err(error) =
                    fs::File::open(parent).and_then(|directory| directory.sync_all())
                {
                    ensure_recovery_required(&coordinator, &guard, operation_id)
                        .map_err(|recovery| format!("{error}; {recovery}"))?;
                    return Err(format!(
                        "The source App Definition was removed but its directory could not be synchronized; recovery is required: {error}"
                    ));
                }
            }
        }
        coordinator
            .transition(&guard, operation_id, OperationPhase::Published)
            .map_err(|error| error.to_string())?;
        if let Err(error) = reconcile(&prepared_changed_paths(&prepared)) {
            coordinator
                .transition(&guard, operation_id, OperationPhase::RecoveryRequired)
                .map_err(|transition| transition.to_string())?;
            return Err(format!(
                "Catalog publication requires session reconciliation: {error}"
            ));
        }
        let record =
            complete_group_publication(&coordinator, &guard, operation_id, &child_operation_ids)
                .map_err(|error| error.to_string())?;
        Ok(status_from_record(
            record,
            prepared.preview.changed_files.clone(),
        ))
    }

    pub fn status(
        &self,
        authored_root: impl AsRef<Path>,
        operation_id: &str,
    ) -> Result<Option<CatalogChangeStatus>, String> {
        let root = fs::canonicalize(authored_root.as_ref()).map_err(|error| error.to_string())?;
        let coordinator = self.coordinator(&root)?;
        let (record, publication_active) = coordinator
            .operation_for_status(operation_id)
            .map_err(|error| error.to_string())?;
        if let Some(record) = record {
            let changed_files = status_changed_files(&root, &record);
            return Ok(Some(status_from_record_with_activity(
                record,
                changed_files,
                publication_active,
            )));
        }
        Ok(None)
    }

    /// Discover unresolved grouped refactors after a complete application relaunch.
    pub fn list_recoveries(
        &self,
        authored_root: impl AsRef<Path>,
    ) -> Result<Vec<CatalogChangeStatus>, String> {
        let root = fs::canonicalize(authored_root.as_ref()).map_err(|error| error.to_string())?;
        let coordinator = self.coordinator(&root)?;
        let records = coordinator
            .list_recoveries()
            .map_err(|error| error.to_string())?;
        let parent_ids = records
            .iter()
            .map(|record| record.operation_id.clone())
            .collect::<std::collections::HashSet<_>>();
        let grouped_child_ids = records
            .iter()
            .flat_map(|record| record.child_operation_ids.iter().cloned())
            .collect::<std::collections::HashSet<_>>();
        Ok(records
            .into_iter()
            .filter(|record| {
                !grouped_child_ids.contains(&record.operation_id)
                    && !record
                        .parent_operation_id
                        .as_deref()
                        .is_some_and(|parent| parent_ids.contains(parent))
            })
            .map(|record| {
                let changed_files = status_changed_files(&root, &record);
                status_from_record(record, changed_files)
            })
            .collect())
    }

    /// Retry one durable grouped publication. Safe children roll forward;
    /// externally changed destinations remain untouched and keep recovery open.
    pub fn retry_recovery(
        &self,
        authored_root: impl AsRef<Path>,
        operation_id: &str,
        reconcile: impl FnOnce(&[PathBuf]) -> Result<(), String>,
    ) -> Result<CatalogChangeStatus, String> {
        let root = fs::canonicalize(authored_root.as_ref()).map_err(|error| error.to_string())?;
        let coordinator = self.coordinator(&root)?;
        let initial = coordinator
            .operation(operation_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "The catalog-change recovery operation is unknown.".to_string())?;
        if coordinator
            .is_group_child_operation(operation_id)
            .map_err(|error| error.to_string())?
        {
            return Err(
                "a grouped child write can only be retried through its parent operation".into(),
            );
        }
        if initial.phase.is_terminal() {
            let changed_files = status_changed_files(&root, &initial);
            return Ok(status_from_record(initial, changed_files));
        }
        let mut reconcile = Some(reconcile);
        if initial.operation_kind == "ordinary-file-write" {
            let record = coordinator
                .retry_recovery(operation_id, |guard, record| {
                    coordinator.publish_staged_file_write_until_reconciled(guard, operation_id)?;
                    let changed_files = record
                        .file_writes
                        .iter()
                        .map(|write| root.join(&write.destination))
                        .collect::<Vec<_>>();
                    if let Some(reconcile) = reconcile.take() {
                        if let Err(error) = reconcile(&changed_files) {
                            coordinator.transition(
                                guard,
                                operation_id,
                                OperationPhase::RecoveryRequired,
                            )?;
                            return Err(
                                crate::catalog_change::CatalogChangeError::InvalidOperation(
                                    format!(
                                        "catalog publication requires session reconciliation: {error}"
                                    ),
                                ),
                            );
                        }
                    }
                    coordinator.transition(guard, operation_id, OperationPhase::Completed)
                })
                .map_err(|error| error.to_string())?;
            let changed_files = status_changed_files(&root, &record);
            return Ok(status_from_record(record, changed_files));
        }
        coordinator
            .retry_recovery(operation_id, |guard, record| {
                let children = coordinator.group_child_operations(guard, operation_id)?;
                let child_ids = children
                    .iter()
                    .map(|child| child.operation_id.as_str())
                    .collect::<std::collections::HashSet<_>>();
                let unknown_children = coordinator
                    .list_recoveries_with_guard(guard)?
                    .into_iter()
                    .filter(|candidate| {
                        candidate.operation_id != operation_id
                            && candidate.operation_kind == "ordinary-file-write"
                            && !child_ids.contains(candidate.operation_id.as_str())
                    })
                    .map(|candidate| candidate.operation_id)
                    .collect::<Vec<_>>();
                if !unknown_children.is_empty() {
                    return Err(
                        crate::catalog_change::CatalogChangeError::RecoveryRequired {
                            operation_ids: unknown_children,
                        },
                    );
                }
                if children.len() < record.child_operation_ids.len() {
                    if matches!(
                        record.phase,
                        OperationPhase::Prepared | OperationPhase::Staged
                    ) {
                        for child in &children {
                            if !child.phase.is_terminal() {
                                coordinator.transition(
                                    guard,
                                    &child.operation_id,
                                    OperationPhase::Rejected,
                                )?;
                            }
                        }
                        return coordinator.transition(
                            guard,
                            operation_id,
                            OperationPhase::Rejected,
                        );
                    }
                    if record.phase != OperationPhase::RecoveryRequired {
                        coordinator.transition(
                            guard,
                            operation_id,
                            OperationPhase::RecoveryRequired,
                        )?;
                    }
                    return Err(
                        crate::catalog_change::CatalogChangeError::RecoveryRequired {
                            operation_ids: vec![operation_id.to_owned()],
                        },
                    );
                }
                if let Some(recovery) = &record.catalog_refactor_recovery {
                    if let Err(error) = validate_recovery_dependency_closure(&root, recovery) {
                        if record.phase != OperationPhase::RecoveryRequired {
                            coordinator.transition(
                                guard,
                                operation_id,
                                OperationPhase::RecoveryRequired,
                            )?;
                        }
                        return Err(crate::catalog_change::CatalogChangeError::InvalidOperation(
                            error,
                        ));
                    }
                }
                let mut durable_parent = record.clone();
                durable_parent.child_operation_ids = children
                    .iter()
                    .map(|child| child.operation_id.clone())
                    .collect();
                let durable_parent = coordinator.update_operation(guard, durable_parent)?;
                match durable_parent.phase {
                    OperationPhase::Prepared => {
                        coordinator.transition(guard, operation_id, OperationPhase::Staged)?;
                        coordinator.transition(guard, operation_id, OperationPhase::Publishing)?;
                    }
                    OperationPhase::Staged => {
                        coordinator.transition(guard, operation_id, OperationPhase::Publishing)?;
                    }
                    _ => {}
                }
                let mut published: Vec<String> = Vec::new();
                for child in &durable_parent.child_operation_ids {
                    if let Err(error) =
                        coordinator.publish_staged_file_write_until_reconciled(guard, child)
                    {
                        if let Err(recovery) = rollback_group_publication(
                            &coordinator,
                            guard,
                            operation_id,
                            &published,
                        ) {
                            return Err(recovery);
                        }
                        return Err(error);
                    }
                    published.push(child.clone());
                }
                if let Some(deletion) = &record.delete_source {
                    let source = root.join(&deletion.source);
                    let deletion_result = (|| match coordinator.snapshot_file(&source)? {
                        Some(snapshot)
                            if digest(&snapshot.bytes) == deletion.expected_digest
                                && deletion
                                    .expected_identity
                                    .as_ref()
                                    .is_none_or(|expected| expected == &snapshot.identity) =>
                        {
                            fs::remove_file(&source)?;
                            if let Some(parent) = source.parent() {
                                fs::File::open(parent)?.sync_all()?;
                            }
                            Ok(())
                        }
                        None => {
                            if let Some(parent) = source.parent() {
                                fs::File::open(parent)?.sync_all()?;
                            }
                            Ok(())
                        }
                        Some(_) => Err(
                            crate::catalog_change::CatalogChangeError::RecoveryRequired {
                                operation_ids: vec![operation_id.to_owned()],
                            },
                        ),
                    })();
                    if let Err(error) = deletion_result {
                        if coordinator.snapshot_file(&source)?.is_none() {
                            ensure_recovery_required(&coordinator, guard, operation_id)?;
                            return Err(error);
                        }
                        if let Err(recovery) = rollback_group_publication(
                            &coordinator,
                            guard,
                            operation_id,
                            &published,
                        ) {
                            return Err(recovery);
                        }
                        return Err(error);
                    }
                }
                coordinator.transition(guard, operation_id, OperationPhase::Published)?;
                let changed_files = absolute_changed_files(&root, record);
                if let Some(reconcile) = reconcile.take() {
                    if let Err(error) = reconcile(&changed_files) {
                        coordinator.transition(
                            guard,
                            operation_id,
                            OperationPhase::RecoveryRequired,
                        )?;
                        return Err(crate::catalog_change::CatalogChangeError::InvalidOperation(
                            format!("catalog publication requires session reconciliation: {error}"),
                        ));
                    }
                }
                complete_group_publication(
                    &coordinator,
                    guard,
                    operation_id,
                    &durable_parent.child_operation_ids,
                )
            })
            .map_err(|error| error.to_string())?;
        let record = coordinator
            .operation(operation_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "The catalog-change recovery record disappeared.".to_string())?;
        let changed_files = status_changed_files(&root, &record);
        Ok(status_from_record(record, changed_files))
    }

    fn coordinator(&self, authored_root: &Path) -> Result<CatalogChangeCoordinator, String> {
        match &self.storage_base {
            Some(storage) => CatalogChangeCoordinator::open_with_storage(authored_root, storage),
            None => CatalogChangeCoordinator::open(authored_root),
        }
        .map_err(|error| error.to_string())
    }

    /// Use an explicit durable journal base for isolated profiles and tests.
    pub(crate) fn with_storage(storage_base: PathBuf) -> Self {
        Self {
            storage_base: Some(storage_base),
            ..Self::default()
        }
    }

    fn find_durable_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<OperationRecord>, String> {
        match &self.storage_base {
            Some(storage_base) => {
                CatalogChangeCoordinator::find_operation_at(storage_base, operation_id)
            }
            None => CatalogChangeCoordinator::find_operation(operation_id),
        }
        .map_err(|error| error.to_string())
    }
}

fn status_from_record(record: OperationRecord, changed_files: Vec<PathBuf>) -> CatalogChangeStatus {
    status_from_record_with_activity(record, changed_files, false)
}

fn authored_relative_path(authored_root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(authored_root)
        .expect("catalog path is inside the authored root")
        .to_path_buf()
}

fn prepared_changed_paths(prepared: &PreparedCatalogChange) -> Vec<PathBuf> {
    let mut paths = prepared
        .files
        .iter()
        .map(|file| file.path.clone())
        .chain(prepared.delete_source.iter().map(|(path, _)| path.clone()))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn status_from_record_with_activity(
    record: OperationRecord,
    changed_files: Vec<PathBuf>,
    publication_active: bool,
) -> CatalogChangeStatus {
    let state = match record.phase {
        OperationPhase::Rejected => CatalogChangeState::Rejected,
        OperationPhase::Completed => CatalogChangeState::Completed,
        OperationPhase::Prepared
        | OperationPhase::Staged
        | OperationPhase::Publishing
        | OperationPhase::Published
        | OperationPhase::RecoveryRequired => {
            if publication_active {
                CatalogChangeState::InProgress
            } else {
                CatalogChangeState::RecoveryRequired
            }
        }
    };
    CatalogChangeStatus {
        operation_id: record.operation_id,
        state,
        changed_files,
    }
}

fn rollback_group_publication(
    coordinator: &CatalogChangeCoordinator,
    guard: &PublicationGuard,
    operation_id: &str,
    published_children: &[String],
) -> Result<(), crate::catalog_change::CatalogChangeError> {
    let mut unresolved_children = Vec::new();
    for child in published_children.iter().rev() {
        if coordinator
            .rollback_staged_file_write(guard, child)
            .is_err()
        {
            unresolved_children.push(child.clone());
        }
        if ensure_recovery_required(coordinator, guard, child).is_err() {
            unresolved_children.push(child.clone());
        }
    }

    let parent = coordinator.operation(operation_id)?.ok_or_else(|| {
        crate::catalog_change::CatalogChangeError::InvalidOperation(format!(
            "unknown parent operation {operation_id:?}"
        ))
    })?;
    if parent.phase != OperationPhase::RecoveryRequired {
        coordinator.transition(guard, operation_id, OperationPhase::RecoveryRequired)?;
    }

    if unresolved_children.is_empty() {
        Ok(())
    } else {
        unresolved_children.push(operation_id.to_owned());
        Err(
            crate::catalog_change::CatalogChangeError::RecoveryRequired {
                operation_ids: unresolved_children,
            },
        )
    }
}

fn complete_group_publication(
    coordinator: &CatalogChangeCoordinator,
    guard: &PublicationGuard,
    operation_id: &str,
    child_operation_ids: &[String],
) -> Result<OperationRecord, crate::catalog_change::CatalogChangeError> {
    for child in child_operation_ids {
        if let Err(error) =
            coordinator.complete_staged_file_write_after_reconciliation(guard, child)
        {
            ensure_recovery_required(coordinator, guard, operation_id)?;
            return Err(error);
        }
    }
    match coordinator.transition(guard, operation_id, OperationPhase::Completed) {
        Ok(record) => Ok(record),
        Err(error) => {
            ensure_recovery_required(coordinator, guard, operation_id)?;
            Err(error)
        }
    }
}

fn ensure_recovery_required(
    coordinator: &CatalogChangeCoordinator,
    guard: &PublicationGuard,
    operation_id: &str,
) -> Result<(), crate::catalog_change::CatalogChangeError> {
    let record = coordinator.operation(operation_id)?.ok_or_else(|| {
        crate::catalog_change::CatalogChangeError::InvalidOperation(format!(
            "unknown operation {operation_id:?}"
        ))
    })?;
    if record.phase != OperationPhase::RecoveryRequired && !record.phase.is_terminal() {
        coordinator.transition(guard, operation_id, OperationPhase::RecoveryRequired)?;
    }
    Ok(())
}

fn validate_recovery_dependency_closure(
    authored_root: &Path,
    recovery: &CatalogRefactorRecovery,
) -> Result<(), String> {
    let expected = recovery
        .dependent_files
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let mut current = Vec::new();
    for path in load_yaml_paths(&authored_root.join("recipes"))? {
        let bytes = fs::read(&path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let recipe = parse_recipe_snapshot(&bytes, &path)?;
        if recovery_references_old_identity(&recipe, &recovery.intent) {
            current.push(authored_relative_path(authored_root, &path));
        }
    }
    current.sort();
    current.dedup();
    let newly_affected = current
        .into_iter()
        .filter(|path| !expected.contains(path))
        .collect::<Vec<_>>();
    if newly_affected.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Catalog dependency closure changed during recovery; resolve new references at {}.",
        newly_affected
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn recovery_references_old_identity(
    recipe: &Recipe,
    intent: &CatalogRefactorRecoveryIntent,
) -> bool {
    match intent {
        CatalogRefactorRecoveryIntent::RenameApp { app_id, .. }
        | CatalogRefactorRecoveryIntent::DeleteApp { app_id } => {
            !discover_app_references(recipe, Path::new("recipe.yaml"), app_id).is_empty()
        }
        CatalogRefactorRecoveryIntent::RenameArtifact {
            app_id,
            artifact_id,
            ..
        } => recipe.artifacts.values().any(|artifact| {
            matches!(
                artifact,
                RecipeArtifact::AppArtifact(reference)
                    if reference.app_ref == *app_id && reference.artifact == *artifact_id
            )
        }),
        CatalogRefactorRecoveryIntent::RenameTarget {
            app_id, target_id, ..
        } => recipe.steps.iter().any(|step| {
            step.app_ref.as_deref() == Some(app_id)
                && (step.params.values().any(
                    |value| matches!(value, ParamValue::AppTarget(target) if target == target_id),
                ) || step
                    .skip_if
                    .iter()
                    .chain(&step.verify)
                    .any(|condition| condition_has_app_target(condition, target_id)))
        }),
    }
}

fn condition_has_app_target(condition: &crate::model::StepCondition, target_id: &str) -> bool {
    matches!(condition.type_name.as_str(), "path_exists" | "file_exists")
        && condition.params.get("path").is_some_and(|value| {
            value.as_object().is_some_and(|object| {
                object.len() == 1
                    && object.get("app_target").and_then(serde_json::Value::as_str)
                        == Some(target_id)
            })
        })
}

fn absolute_changed_files(root: &Path, record: &OperationRecord) -> Vec<PathBuf> {
    record
        .changed_files
        .iter()
        .map(|path| root.join(path))
        .collect()
}

fn status_changed_files(_root: &Path, record: &OperationRecord) -> Vec<PathBuf> {
    if record.changed_files.is_empty() {
        record
            .file_writes
            .iter()
            .map(|write| write.destination.clone())
            .collect()
    } else {
        record.changed_files.clone()
    }
}

fn verify_prepared_snapshot(
    coordinator: &CatalogChangeCoordinator,
    prepared: &PreparedCatalogChange,
) -> Result<(), String> {
    let mut current_inventory = load_yaml_paths(&prepared.authored_root.join("apps"))?;
    current_inventory.extend(load_yaml_paths(&prepared.authored_root.join("recipes"))?);
    current_inventory.sort();
    if current_inventory != prepared.inventory_paths {
        return Err("The authored catalog inventory changed after preview.".into());
    }
    for (path, expected) in &prepared.observed_files {
        if coordinator
            .snapshot_file(path)
            .map_err(|error| error.to_string())?
            .as_ref()
            != Some(expected)
        {
            return Err(format!(
                "The catalog changed after preview at {}.",
                path.display()
            ));
        }
    }
    for file in &prepared.files {
        if coordinator
            .snapshot_file(&file.path)
            .map_err(|error| error.to_string())?
            != file.original_snapshot
        {
            return Err(format!(
                "The catalog changed after preview at {}.",
                file.path.display()
            ));
        }
    }
    if let Some((path, expected)) = &prepared.delete_source {
        if coordinator
            .snapshot_file(path)
            .map_err(|_| {
                format!(
                    "The source catalog file changed after preview at {}.",
                    path.display()
                )
            })?
            .as_ref()
            != Some(expected)
        {
            return Err(format!(
                "The source catalog file changed after preview at {}.",
                path.display()
            ));
        }
    }
    Ok(())
}

fn validate_staged_candidates(
    coordinator: &CatalogChangeCoordinator,
    guard: &PublicationGuard,
    child_operation_ids: &[String],
    prepared: &PreparedCatalogChange,
) -> Result<(), String> {
    let mut staged_files = Vec::new();
    for operation_id in child_operation_ids {
        let (_, snapshot) = coordinator
            .staged_file_snapshot(guard, operation_id)
            .map_err(|error| error.to_string())?;
        let record = coordinator
            .operation(operation_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Staged operation {operation_id:?} disappeared."))?;
        let write = record
            .file_writes
            .first()
            .ok_or_else(|| format!("Staged operation {operation_id:?} has no file."))?;
        let canonical = match write.destination.components().next() {
            Some(std::path::Component::Normal(name)) if name == "apps" => {
                let source = std::str::from_utf8(&snapshot.bytes)
                    .map_err(|error| format!("Staged App Definition is not UTF-8: {error}"))?;
                let app = crate::authored_models::parse_app_definition_yaml(source)
                    .map_err(|error| format!("Staged App Definition is invalid: {error}"))?;
                emit_app_definition_yaml(&app)
                    .map_err(|error| format!("Staged App Definition is invalid: {error}"))?
            }
            Some(std::path::Component::Normal(name)) if name == "recipes" => {
                let recipe = parse_recipe_snapshot(&snapshot.bytes, &write.destination).map_err(
                    |error| {
                        format!(
                            "Staged Recipe {} is invalid: {error}",
                            write.destination.display()
                        )
                    },
                )?;
                yaml::emit_recipe_yaml(&recipe).map_err(|error| {
                    format!(
                        "Staged Recipe {} is invalid: {error}",
                        write.destination.display()
                    )
                })?
            }
            _ => {
                return Err(format!(
                    "Staged catalog candidate {} has an unsupported destination.",
                    write.destination.display()
                ));
            }
        };
        if canonical.as_bytes() != snapshot.bytes.as_slice() {
            return Err(format!(
                "Staged catalog candidate {} is not canonical.",
                write.destination.display()
            ));
        }
        staged_files.push((write.destination.clone(), snapshot.bytes.clone()));
    }
    validate_staged_catalog_overlay(prepared, &staged_files)
}

fn validate_staged_catalog_overlay(
    prepared: &PreparedCatalogChange,
    staged_files: &[(PathBuf, Vec<u8>)],
) -> Result<(), String> {
    let apps_directory = prepared.authored_root.join("apps");
    let recipes_directory = prepared.authored_root.join("recipes");
    let mut catalog_files = prepared
        .observed_files
        .iter()
        .map(|(path, snapshot)| (path.clone(), snapshot.bytes.clone()))
        .collect::<BTreeMap<_, _>>();
    if let Some((path, _)) = &prepared.delete_source {
        catalog_files.remove(path);
    }

    let mut changed_apps = HashMap::new();
    let mut changed_ids = HashSet::new();
    let mut staged_recipe_paths = HashSet::new();
    for (path, bytes) in staged_files {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            prepared.authored_root.join(path)
        };
        catalog_files.insert(path.clone(), bytes.clone());
        if path.parent() == Some(apps_directory.as_path()) {
            let source = std::str::from_utf8(bytes)
                .map_err(|error| format!("Staged App Definition is not UTF-8: {error}"))?;
            let app = crate::authored_models::parse_app_definition_yaml(source)
                .map_err(|error| format!("Staged App Definition is invalid: {error}"))?;
            changed_ids.insert(app.id.clone());
            changed_apps.insert(path, app);
        } else if path.parent() == Some(recipes_directory.as_path()) {
            staged_recipe_paths.insert(path);
        }
    }
    if let Some((path, snapshot)) = &prepared.delete_source {
        if let Ok(source) = std::str::from_utf8(&snapshot.bytes) {
            if let Ok(app) = crate::authored_models::parse_app_definition_yaml(source) {
                changed_ids.insert(app.id.clone());
            }
        }
        catalog_files.remove(path);
    }

    let mut app_identities = Vec::new();
    for (path, bytes) in catalog_files
        .iter()
        .filter(|(path, _)| path.parent() == Some(apps_directory.as_path()))
    {
        if let Some(app) = changed_apps.get(path) {
            app_identities.push((path.clone(), app.id.clone(), Some(app.package_id.clone())));
            continue;
        }
        let source = match std::str::from_utf8(bytes) {
            Ok(source) => source,
            Err(_) => continue,
        };
        let Ok(document) = serde_yaml::from_str::<serde_yaml::Value>(source) else {
            continue;
        };
        let Some(id) = document.get("id").and_then(serde_yaml::Value::as_str) else {
            continue;
        };
        let package_id = document
            .get("package_id")
            .and_then(serde_yaml::Value::as_str)
            .map(str::to_owned);
        app_identities.push((path.clone(), id.to_owned(), package_id));
    }

    for (path, app) in &changed_apps {
        let expected = apps_directory.join(format!("{}.yaml", app.id));
        if &expected != path {
            return Err(format!(
                "Staged App Definition {} does not use its canonical apps/<id>.yaml path.",
                authored_relative_path(&prepared.authored_root, path).display()
            ));
        }
        for (other_path, other_id, other_package) in &app_identities {
            if other_path == path {
                continue;
            }
            if other_id == &app.id {
                return Err(format!(
                    "Staged App Definition {} conflicts with App ID '{}' in {}.",
                    authored_relative_path(&prepared.authored_root, path).display(),
                    app.id,
                    authored_relative_path(&prepared.authored_root, other_path).display()
                ));
            }
            if other_package.as_deref() == Some(app.package_id.as_str()) {
                return Err(format!(
                    "Staged App Definition {} conflicts with package '{}' in {}.",
                    authored_relative_path(&prepared.authored_root, path).display(),
                    app.package_id,
                    authored_relative_path(&prepared.authored_root, other_path).display()
                ));
            }
        }
    }

    for (path, bytes) in catalog_files
        .iter()
        .filter(|(path, _)| path.parent() == Some(recipes_directory.as_path()))
    {
        let recipe = match parse_recipe_snapshot(bytes, path) {
            Ok(recipe) => recipe,
            Err(error) if staged_recipe_paths.contains(path) => {
                return Err(format!(
                    "Staged Recipe {} is invalid: {error}",
                    authored_relative_path(&prepared.authored_root, path).display()
                ));
            }
            Err(_) => continue,
        };
        let affected = staged_recipe_paths.contains(path)
            || changed_ids
                .iter()
                .any(|app_id| !discover_app_references(&recipe, path, app_id).is_empty());
        if !affected {
            continue;
        }
        let relative_path = authored_relative_path(&prepared.authored_root, path);
        let mut affected_steps = prepared
            .preview
            .references
            .iter()
            .filter(|reference| reference.path == relative_path)
            .filter_map(|reference| {
                reference
                    .object
                    .strip_prefix("steps.")
                    .and_then(|rest| rest.split('.').next())
                    .map(str::to_owned)
            })
            .collect::<HashSet<_>>();
        affected_steps.extend(
            recipe
                .steps
                .iter()
                .filter(|step| {
                    step.app_ref
                        .as_deref()
                        .is_some_and(|app_id| changed_ids.contains(app_id))
                })
                .map(|step| step.id.clone()),
        );
        let mut affected_artifacts = prepared
            .preview
            .references
            .iter()
            .filter(|reference| reference.path == relative_path)
            .filter_map(|reference| {
                reference
                    .object
                    .strip_prefix("artifacts.")
                    .map(str::to_owned)
            })
            .collect::<HashSet<_>>();
        affected_artifacts.extend(recipe.artifacts.iter().filter_map(|(id, artifact)| {
            matches!(
                artifact,
                RecipeArtifact::AppArtifact(reference)
                    if changed_ids.contains(&reference.app_ref)
            )
            .then(|| id.clone())
        }));

        let mut definitions = Vec::new();
        for app_id in recipe_app_reference_ids(&recipe) {
            let Some((_app_path, app_bytes)) = app_identities
                .iter()
                .find(|(_, id, _)| id == &app_id)
                .and_then(|(app_path, _, _)| {
                    catalog_files.get(app_path).map(|bytes| (app_path, bytes))
                })
            else {
                continue;
            };
            let source = match std::str::from_utf8(app_bytes) {
                Ok(source) => source,
                Err(error) if changed_ids.contains(&app_id) => {
                    return Err(format!(
                        "Staged Recipe {} references a changed App Definition that is not UTF-8: {error}",
                        relative_path.display()
                    ));
                }
                Err(_) => continue,
            };
            match crate::authored_models::parse_app_definition_yaml(source) {
                Ok(app) => definitions.push(app),
                Err(error) if changed_ids.contains(&app_id) => {
                    return Err(format!(
                        "Staged Recipe {} references changed App '{}' but its Definition is invalid: {error}",
                        relative_path.display(),
                        app_id
                    ));
                }
                Err(_) => continue,
            }
        }
        let mut errors = crate::planner::validate_recipe_app_authority(&recipe, &definitions)
            .into_iter()
            .filter(|error| {
                authority_error_is_in_changed_closure(
                    error,
                    &changed_ids,
                    &affected_steps,
                    &affected_artifacts,
                )
            })
            .map(|error| format!("{}: {}", error.code, error.message))
            .collect::<Vec<_>>();
        for step in recipe.steps.iter().filter(|step| {
            step.type_name == "grant_permissions" && affected_steps.contains(&step.id)
        }) {
            let Some(app) = step
                .app_ref
                .as_deref()
                .and_then(|id| definitions.iter().find(|app| app.id == id))
            else {
                continue;
            };
            errors.extend(
                crate::app_definition_admission::permission_selection_violations(app, step)
                    .into_iter()
                    .map(|violation| format!("{}: {}", violation.code, violation.message)),
            );
        }
        if !errors.is_empty() {
            return Err(format!(
                "Staged Recipe {} fails App authority validation: {}.",
                relative_path.display(),
                errors.join("; ")
            ));
        }
    }
    Ok(())
}

fn recipe_app_reference_ids(recipe: &Recipe) -> HashSet<String> {
    recipe
        .artifacts
        .values()
        .filter_map(|artifact| match artifact {
            RecipeArtifact::AppArtifact(reference) => Some(reference.app_ref.clone()),
            RecipeArtifact::RemoteFile(_) => None,
        })
        .chain(recipe.steps.iter().filter_map(|step| step.app_ref.clone()))
        .collect()
}

fn authority_error_is_in_changed_closure(
    error: &crate::planner::PlannerMessage,
    changed_ids: &HashSet<String>,
    affected_steps: &HashSet<String>,
    affected_artifacts: &HashSet<String>,
) -> bool {
    ["app_id", "app_ref"].iter().any(|key| {
        error
            .details
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| changed_ids.contains(value))
    }) || error
        .details
        .get("step_id")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| affected_steps.contains(value))
        || error
            .details
            .get("artifact_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| affected_artifacts.contains(value))
}

fn prepare_catalog_change(
    coordinator: &CatalogChangeCoordinator,
    authored_root: &Path,
    intent: CatalogChangeIntent,
) -> Result<PreparedCatalogChange, String> {
    let authored_root = fs::canonicalize(authored_root)
        .map_err(|error| format!("The authored root cannot be opened: {error}"))?;
    validate_intent(&intent)?;
    let apps_directory = authored_root.join("apps");
    let recipes_directory = authored_root.join("recipes");
    let owner_id = intent_owner(&intent);
    let app_paths = load_yaml_paths(&apps_directory)?;
    let apps = app_paths
        .iter()
        .cloned()
        .into_iter()
        .map(|path| {
            let snapshot = coordinator
                .snapshot_file(&path)
                .map_err(|error| format!("Failed to read {}: {error}", path.display()))?
                .ok_or_else(|| format!("App Definition {} disappeared.", path.display()))?;
            let source = std::str::from_utf8(&snapshot.bytes).map_err(|error| {
                format!(
                    "App references cannot be classified in {}: {error}",
                    path.display()
                )
            })?;
            let document: serde_yaml::Value = serde_yaml::from_str(source).map_err(|error| {
                format!(
                    "App identity cannot be classified in {}: {error}",
                    path.display()
                )
            })?;
            let app_id = document
                .get("id")
                .and_then(serde_yaml::Value::as_str)
                .ok_or_else(|| format!("App identity cannot be classified in {}.", path.display()))?
                .to_owned();
            let app = if app_id == owner_id {
                Some(
                    crate::authored_models::parse_app_definition_yaml(source).map_err(|error| {
                        format!(
                            "The requested App Definition {} is invalid: {error}",
                            path.display()
                        )
                    })?,
                )
            } else {
                None
            };
            Ok((path, snapshot, app_id, app))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut inventory_paths = app_paths;
    let mut observed_files = apps
        .iter()
        .map(|(path, snapshot, _, _)| (path.clone(), snapshot.clone()))
        .collect::<Vec<_>>();
    let owner_indices = apps
        .iter()
        .enumerate()
        .filter_map(|(index, (_, _, app_id, _))| (app_id == &owner_id).then_some(index))
        .collect::<Vec<_>>();
    if owner_indices.len() != 1 {
        return Err(format!(
            "App identity {owner_id:?} must resolve to exactly one App Definition."
        ));
    }
    let owner_index = owner_indices[0];
    let (owner_path, owner_snapshot, _, owner) = &apps[owner_index];
    let owner = owner
        .as_ref()
        .ok_or_else(|| "The requested App Definition could not be parsed.".to_string())?;
    let mut changed_files = Vec::new();
    let mut references = Vec::new();
    let mut prepared_files = Vec::new();
    let mut delete_source = None;

    if let CatalogChangeIntent::RenameApp { new_id, .. } = &intent {
        if apps.iter().any(|(_, _, app_id, _)| app_id == new_id) {
            return Err(format!("App identity {new_id:?} already exists."));
        }
    }
    match &intent {
        CatalogChangeIntent::RenameArtifact {
            artifact_id,
            new_id,
            ..
        } => {
            if !owner.artifacts.contains_key(artifact_id) {
                return Err(format!("App Artifact {artifact_id:?} does not exist."));
            }
            if owner.artifacts.contains_key(new_id) {
                return Err(format!("App Artifact {new_id:?} already exists."));
            }
        }
        CatalogChangeIntent::RenameTarget {
            target_id, new_id, ..
        } => {
            if !owner.targets.contains_key(target_id) {
                return Err(format!("App Target {target_id:?} does not exist."));
            }
            if owner.targets.contains_key(new_id) {
                return Err(format!("App Target {new_id:?} already exists."));
            }
        }
        _ => {}
    }

    let mut updated_owner = owner.clone();
    if !rewrite_app_definition(&mut updated_owner, &intent) {
        return Err("The requested App identity change cannot be applied.".into());
    }
    let owner_destination = match &intent {
        CatalogChangeIntent::RenameApp { new_id, .. } => {
            let destination = apps_directory.join(format!("{new_id}.yaml"));
            if coordinator
                .snapshot_file(&destination)
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err(format!(
                    "The App Definition destination {} already exists.",
                    destination.display()
                ));
            }
            destination
        }
        _ => owner_path.clone(),
    };
    if !matches!(intent, CatalogChangeIntent::DeleteApp { .. }) {
        let bytes = emit_app_definition_yaml(&updated_owner)
            .map_err(|error| format!("The updated App Definition is invalid: {error}"))?
            .into_bytes();
        let original_snapshot = if owner_destination == *owner_path {
            Some(owner_snapshot.clone())
        } else {
            None
        };
        prepared_files.push(PreparedFile {
            path: owner_destination.clone(),
            original_snapshot,
            permission_source: Some(owner_snapshot.clone()),
            bytes,
        });
        changed_files.push(owner_destination.clone());
    }

    let recipe_paths = load_yaml_paths(&recipes_directory)?;
    inventory_paths.extend(recipe_paths.iter().cloned());
    for path in recipe_paths {
        let original_snapshot = coordinator
            .snapshot_file(&path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?
            .ok_or_else(|| format!("Recipe {} disappeared.", path.display()))?;
        observed_files.push((path.clone(), original_snapshot.clone()));
        let original = &original_snapshot.bytes;
        match parse_recipe_snapshot(original, &path) {
            Ok(mut recipe) => {
                let mut sites = if matches!(intent, CatalogChangeIntent::DeleteApp { .. }) {
                    discover_app_references(&recipe, &path, owner_id)
                } else {
                    rewrite_recipe(&mut recipe, path.clone(), &intent)
                };
                if sites.is_empty() {
                    continue;
                }
                references.append(&mut sites);
                if matches!(intent, CatalogChangeIntent::DeleteApp { .. }) {
                    continue;
                }
                let bytes = yaml::emit_recipe_yaml(&recipe)
                    .map_err(|error| {
                        format!("Updated Recipe {} is invalid: {error}", path.display())
                    })?
                    .into_bytes();
                if bytes.as_slice() != original.as_slice() {
                    prepared_files.push(PreparedFile {
                        path: path.clone(),
                        original_snapshot: Some(original_snapshot.clone()),
                        permission_source: Some(original_snapshot),
                        bytes,
                    });
                    changed_files.push(path);
                }
            }
            Err(error) => {
                let raw: serde_yaml::Value = serde_yaml::from_slice(original).map_err(|_| {
                    format!(
                        "Recipe references cannot be classified in {}: {error}",
                        path.display()
                    )
                })?;
                if raw_mentions_app(&raw, owner_id) {
                    return Err(format!(
                        "Recipe references cannot be classified in {}: {error}",
                        path.display()
                    ));
                }
            }
        }
    }

    if matches!(intent, CatalogChangeIntent::DeleteApp { .. }) {
        if !references.is_empty() {
            let locations = references
                .iter()
                .map(|reference| {
                    format!(
                        "{} ({}.{})",
                        reference.path.display(),
                        reference.object,
                        reference.field
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            return Err(format!(
                "App {owner_id:?} is referenced at {} authored locations: {locations}.",
                references.len(),
            ));
        }
        delete_source = Some((owner_path.clone(), owner_snapshot.clone()));
        changed_files.push(owner_path.clone());
    } else if owner_destination != *owner_path {
        delete_source = Some((owner_path.clone(), owner_snapshot.clone()));
        changed_files.push(owner_path.clone());
        references.push(site(owner_path, format!("apps.{owner_id}"), "id"));
    }

    changed_files.sort();
    changed_files.dedup();
    inventory_paths.sort();
    references.sort_by(|left, right| {
        (&left.path, &left.object, &left.field).cmp(&(&right.path, &right.object, &right.field))
    });
    let snapshot_digest = snapshot_digest(&observed_files, &prepared_files, delete_source.as_ref());
    let sequence = NEXT_OPERATION.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let operation_id = catalog_change_operation_id(timestamp, std::process::id(), sequence);
    let preview_token =
        digest(format!("{operation_id}\0{snapshot_digest}\0{:?}", intent).as_bytes());
    let references = references
        .into_iter()
        .map(|mut reference| {
            reference.path = authored_relative_path(&authored_root, &reference.path);
            reference
        })
        .collect();
    let changed_files = changed_files
        .into_iter()
        .map(|path| authored_relative_path(&authored_root, &path))
        .collect();
    let preview = CatalogChangePreview {
        operation_id,
        preview_token,
        intent,
        references,
        changed_files,
        blockers: Vec::new(),
        snapshot_digest,
    };
    Ok(PreparedCatalogChange {
        preview,
        authored_root,
        files: prepared_files,
        delete_source,
        observed_files,
        inventory_paths,
        binding: None,
    })
}

fn validate_intent(intent: &CatalogChangeIntent) -> Result<(), String> {
    let ids: Vec<&str> = match intent {
        CatalogChangeIntent::RenameApp { app_id, new_id } => vec![app_id, new_id],
        CatalogChangeIntent::RenameArtifact {
            app_id,
            artifact_id,
            new_id,
        } => vec![app_id, artifact_id, new_id],
        CatalogChangeIntent::RenameTarget {
            app_id,
            target_id,
            new_id,
        } => vec![app_id, target_id, new_id],
        CatalogChangeIntent::DeleteApp { app_id } => vec![app_id],
    };
    if ids.iter().all(|id| is_valid_identifier(id)) {
        Ok(())
    } else {
        Err("Catalog identities must use authored identifier syntax.".into())
    }
}

fn intent_owner(intent: &CatalogChangeIntent) -> &str {
    match intent {
        CatalogChangeIntent::RenameApp { app_id, .. }
        | CatalogChangeIntent::RenameArtifact { app_id, .. }
        | CatalogChangeIntent::RenameTarget { app_id, .. }
        | CatalogChangeIntent::DeleteApp { app_id } => app_id,
    }
}

fn durable_recovery_intent(intent: &CatalogChangeIntent) -> CatalogRefactorRecoveryIntent {
    match intent.clone() {
        CatalogChangeIntent::RenameApp { app_id, new_id } => {
            CatalogRefactorRecoveryIntent::RenameApp { app_id, new_id }
        }
        CatalogChangeIntent::RenameArtifact {
            app_id,
            artifact_id,
            new_id,
        } => CatalogRefactorRecoveryIntent::RenameArtifact {
            app_id,
            artifact_id,
            new_id,
        },
        CatalogChangeIntent::RenameTarget {
            app_id,
            target_id,
            new_id,
        } => CatalogRefactorRecoveryIntent::RenameTarget {
            app_id,
            target_id,
            new_id,
        },
        CatalogChangeIntent::DeleteApp { app_id } => {
            CatalogRefactorRecoveryIntent::DeleteApp { app_id }
        }
    }
}

fn discover_app_references(recipe: &Recipe, path: &Path, app_id: &str) -> Vec<ReferenceSite> {
    let mut clone = recipe.clone();
    rewrite_recipe(
        &mut clone,
        path.to_path_buf(),
        &CatalogChangeIntent::RenameApp {
            app_id: app_id.to_owned(),
            new_id: "__deleted_app__".into(),
        },
    )
}

fn load_yaml_paths(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let directory_metadata = fs::symlink_metadata(directory)
        .map_err(|error| format!("Failed to inspect {}: {error}", directory.display()))?;
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err(format!(
            "Catalog directory {} must be a real directory.",
            directory.display()
        ));
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("Failed to scan {}: {error}", directory.display()))?
    {
        let entry =
            entry.map_err(|error| format!("Failed to scan {}: {error}", directory.display()))?;
        let path = entry.path();
        let is_yaml = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension, "yaml" | "yml"));
        if !is_yaml {
            continue;
        }
        if !entry
            .file_type()
            .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?
            .is_file()
        {
            return Err(format!(
                "Catalog file {} must be a regular file, not a link or special file.",
                path.display()
            ));
        }
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

fn raw_mentions_app(value: &serde_yaml::Value, app_id: &str) -> bool {
    match value {
        serde_yaml::Value::Mapping(mapping) => mapping.iter().any(|(key, value)| {
            (key.as_str() == Some("app_ref") && value.as_str() == Some(app_id))
                || raw_mentions_app(value, app_id)
        }),
        serde_yaml::Value::Sequence(items) => {
            items.iter().any(|value| raw_mentions_app(value, app_id))
        }
        _ => false,
    }
}

fn parse_recipe_snapshot(bytes: &[u8], path: &Path) -> Result<Recipe, String> {
    let value: serde_yaml::Value = serde_yaml::from_slice(bytes)
        .map_err(|error| format!("Recipe YAML cannot be parsed: {error}"))?;
    let mapping = match value {
        serde_yaml::Value::Mapping(mapping) => mapping,
        _ => return Err("Recipe YAML must contain a top-level mapping.".into()),
    };
    let recipe = yaml::parse_recipe_mapping(&mapping, path).map_err(|error| error.to_string())?;
    if let Some(issue) = yaml::unsupported_step_issue(&recipe) {
        return Err(issue.message);
    }
    Ok(recipe)
}

fn snapshot_digest(
    observed_files: &[(PathBuf, FileSnapshot)],
    files: &[PreparedFile],
    delete_source: Option<&(PathBuf, FileSnapshot)>,
) -> String {
    let mut hasher = Sha256::new();
    for (path, snapshot) in observed_files {
        hasher.update(path.as_os_str().to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(digest(&snapshot.bytes).as_bytes());
        hasher.update(
            serde_json::to_vec(&snapshot.identity).expect("filesystem identity must serialize"),
        );
    }
    hasher.update([0xff]);
    for file in files {
        hasher.update(file.path.as_os_str().to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(digest(&file.bytes).as_bytes());
    }
    if let Some((path, snapshot)) = delete_source {
        hasher.update(path.as_os_str().to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(digest(&snapshot.bytes).as_bytes());
        hasher.update(
            serde_json::to_vec(&snapshot.identity).expect("filesystem identity must serialize"),
        );
    }
    hex(&hasher.finalize())
}

fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

/// Rewrite one Recipe and return only the typed locations that changed.
pub fn rewrite_recipe(
    recipe: &mut Recipe,
    path: PathBuf,
    intent: &CatalogChangeIntent,
) -> Vec<ReferenceSite> {
    let mut sites = Vec::new();
    for (local_id, artifact) in &mut recipe.artifacts {
        let RecipeArtifact::AppArtifact(reference) = artifact else {
            continue;
        };
        match intent {
            CatalogChangeIntent::RenameApp { app_id, new_id } if reference.app_ref == *app_id => {
                reference.app_ref = new_id.clone();
                sites.push(site(&path, format!("artifacts.{local_id}"), "app_ref"));
            }
            CatalogChangeIntent::RenameArtifact {
                app_id,
                artifact_id,
                new_id,
            } if reference.app_ref == *app_id && reference.artifact == *artifact_id => {
                reference.artifact = new_id.clone();
                sites.push(site(&path, format!("artifacts.{local_id}"), "artifact"));
            }
            _ => {}
        }
    }

    for step in &mut recipe.steps {
        if let CatalogChangeIntent::RenameApp { app_id, new_id } = intent {
            if step.app_ref.as_deref() == Some(app_id) {
                step.app_ref = Some(new_id.clone());
                sites.push(site(&path, format!("steps.{}", step.id), "app_ref"));
            }
        }
        let CatalogChangeIntent::RenameTarget {
            app_id,
            target_id,
            new_id,
        } = intent
        else {
            continue;
        };
        if step.app_ref.as_deref() != Some(app_id) {
            continue;
        }
        for (parameter, value) in &mut step.params {
            if matches!(value, ParamValue::AppTarget(value) if value == target_id) {
                *value = ParamValue::AppTarget(new_id.clone());
                sites.push(site(
                    &path,
                    format!("steps.{}", step.id),
                    format!("params.{parameter}"),
                ));
            }
        }
        rewrite_target_conditions(
            &mut step.skip_if,
            &path,
            &step.id,
            "skip_if",
            target_id,
            new_id,
            &mut sites,
        );
        rewrite_target_conditions(
            &mut step.verify,
            &path,
            &step.id,
            "verify",
            target_id,
            new_id,
            &mut sites,
        );
    }
    sites
}

/// Rewrite the owning App Definition. Returns false when the source identity
/// is absent, allowing callers to reject before staging any file.
pub fn rewrite_app_definition(app: &mut AppDefinitionV1, intent: &CatalogChangeIntent) -> bool {
    match intent {
        CatalogChangeIntent::RenameApp { app_id, new_id } if app.id == *app_id => {
            app.id = new_id.clone();
            true
        }
        CatalogChangeIntent::RenameArtifact {
            app_id,
            artifact_id,
            new_id,
        } if app.id == *app_id => rename_key(&mut app.artifacts, artifact_id, new_id),
        CatalogChangeIntent::RenameTarget {
            app_id,
            target_id,
            new_id,
        } if app.id == *app_id => rename_key(&mut app.targets, target_id, new_id),
        CatalogChangeIntent::DeleteApp { app_id } => app.id == *app_id,
        _ => false,
    }
}

fn rewrite_target_conditions(
    conditions: &mut [crate::model::StepCondition],
    path: &PathBuf,
    step_id: &str,
    collection: &str,
    target_id: &str,
    new_id: &str,
    sites: &mut Vec<ReferenceSite>,
) {
    for (index, condition) in conditions.iter_mut().enumerate() {
        if !matches!(condition.type_name.as_str(), "path_exists" | "file_exists") {
            continue;
        }
        let Some(serde_json::Value::Object(object)) = condition.params.get_mut("path") else {
            continue;
        };
        if object.len() != 1
            || object.get("app_target").and_then(serde_json::Value::as_str) != Some(target_id)
        {
            continue;
        }
        object.insert(
            "app_target".into(),
            serde_json::Value::String(new_id.to_owned()),
        );
        sites.push(site(
            path,
            format!("steps.{step_id}.{collection}[{index}]"),
            "params.path",
        ));
    }
}

fn rename_key<T>(values: &mut IndexMap<String, T>, source: &str, destination: &str) -> bool {
    if source == destination || values.contains_key(destination) {
        return false;
    }
    let Some(index) = values.get_index_of(source) else {
        return false;
    };
    let Some((_, value)) = values.shift_remove_entry(source) else {
        return false;
    };
    values.shift_insert(index, destination.to_owned(), value);
    true
}

fn site(path: &PathBuf, object: String, field: impl Into<String>) -> ReferenceSite {
    ReferenceSite {
        path: path.clone(),
        object,
        field: field.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authored_models::{
        AppDefinitionV1, OrderedValueMap, APP_DEFINITION_KIND, SCHEMA_VERSION_V1,
    };
    use crate::model::{
        AppArtifactReference, OrderedMap, ParamValue, RecipeArtifact, RecipeProvides, Step,
        StepCondition, StepConstraints,
    };
    use serde_json::json;

    #[test]
    fn operation_ids_distinguish_processes_and_counter_restarts() {
        assert_ne!(
            catalog_change_operation_id(100, 7, 1),
            catalog_change_operation_id(100, 8, 1)
        );
        assert_ne!(
            catalog_change_operation_id(100, 7, 1),
            catalog_change_operation_id(101, 7, 1)
        );
    }

    #[test]
    fn app_rename_changes_only_typed_app_references() {
        let mut recipe = recipe();
        let sites = rewrite_recipe(
            &mut recipe,
            PathBuf::from("recipes/example.yaml"),
            &CatalogChangeIntent::RenameApp {
                app_id: "old".into(),
                new_id: "new".into(),
            },
        );
        let RecipeArtifact::AppArtifact(reference) = &recipe.artifacts["local"] else {
            panic!("expected app artifact")
        };
        assert_eq!(reference.app_ref, "new");
        assert_eq!(recipe.steps[0].app_ref.as_deref(), Some("new"));
        assert_eq!(recipe.steps[0].name, "old");
        assert_eq!(sites.len(), 2);
    }

    #[test]
    fn artifact_rename_preserves_recipe_local_artifact_identity() {
        let mut recipe = recipe();
        let sites = rewrite_recipe(
            &mut recipe,
            PathBuf::from("recipes/example.yaml"),
            &CatalogChangeIntent::RenameArtifact {
                app_id: "old".into(),
                artifact_id: "apk".into(),
                new_id: "release".into(),
            },
        );
        assert!(recipe.artifacts.contains_key("local"));
        let RecipeArtifact::AppArtifact(reference) = &recipe.artifacts["local"] else {
            panic!("expected app artifact")
        };
        assert_eq!(reference.artifact, "release");
        assert_eq!(sites[0].field, "artifact");
    }

    #[test]
    fn target_rename_changes_params_and_conditions_only_in_owner_context() {
        let mut recipe = recipe();
        recipe.steps.push(Step {
            app_ref: Some("other".into()),
            ..recipe.steps[0].clone()
        });
        let sites = rewrite_recipe(
            &mut recipe,
            PathBuf::from("recipes/example.yaml"),
            &CatalogChangeIntent::RenameTarget {
                app_id: "old".into(),
                target_id: "config".into(),
                new_id: "settings".into(),
            },
        );
        assert!(matches!(
            recipe.steps[0].params["destination"],
            ParamValue::AppTarget(ref value) if value == "settings"
        ));
        assert_eq!(
            recipe.steps[0].skip_if[0].params["path"]["app_target"],
            json!("settings")
        );
        assert!(matches!(
            recipe.steps[1].params["destination"],
            ParamValue::AppTarget(ref value) if value == "config"
        ));
        assert_eq!(sites.len(), 3);
    }

    #[test]
    fn target_rename_rewrites_only_schema_defined_condition_paths() {
        let mut recipe = recipe();
        recipe.steps[0].params.insert(
            "opaque".into(),
            ParamValue::Literal(json!({"nested": {"app_target": "config"}})),
        );
        recipe.steps[0].skip_if.push(StepCondition {
            type_name: "path_exists".into(),
            params: OrderedMap::from([("other".into(), json!({"app_target": "config"}))]),
        });
        recipe.steps[0].skip_if.push(StepCondition {
            type_name: "unknown_condition".into(),
            params: OrderedMap::from([("path".into(), json!({"app_target": "config"}))]),
        });
        recipe.steps[0].skip_if.push(StepCondition {
            type_name: "path_exists".into(),
            params: OrderedMap::from([(
                "path".into(),
                json!({
                    "app_target": "config",
                    "nested": {"app_target": "config"}
                }),
            )]),
        });

        let sites = rewrite_recipe(
            &mut recipe,
            PathBuf::from("recipes/example.yaml"),
            &CatalogChangeIntent::RenameTarget {
                app_id: "old".into(),
                target_id: "config".into(),
                new_id: "settings".into(),
            },
        );

        assert_eq!(
            recipe.steps[0].skip_if[0].params["path"]["app_target"],
            json!("settings")
        );
        assert_eq!(
            recipe.steps[0].verify[0].params["path"]["app_target"],
            json!("settings")
        );
        assert_eq!(
            recipe.steps[0].params["opaque"],
            ParamValue::Literal(json!({"nested": {"app_target": "config"}}))
        );
        assert_eq!(
            recipe.steps[0].skip_if[1].params["other"]["app_target"],
            json!("config")
        );
        assert_eq!(
            recipe.steps[0].skip_if[2].params["path"]["app_target"],
            json!("config")
        );
        assert_eq!(
            recipe.steps[0].skip_if[3].params["path"]["app_target"],
            json!("config")
        );
        assert_eq!(sites.len(), 3);
    }

    #[test]
    fn unrelated_semantically_invalid_app_does_not_block_scoped_rename() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let owner =
            include_str!("../../../authored/apps/armsx1.yaml").replace("id: armsx1", "id: owner");
        fs::write(apps.join("owner.yaml"), owner).unwrap();
        fs::write(apps.join("unrelated.yaml"), "id: unrelated\n").unwrap();

        let preview = CatalogRefactorSession::default()
            .preview(
                root.path(),
                CatalogChangeIntent::RenameApp {
                    app_id: "owner".into(),
                    new_id: "renamed".into(),
                },
            )
            .unwrap();

        assert_eq!(
            preview.changed_files,
            vec![
                PathBuf::from("apps/owner.yaml"),
                PathBuf::from("apps/renamed.yaml")
            ]
        );
    }

    #[test]
    fn staged_overlay_rejects_dependent_recipe_authority_errors_before_publication() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        let recipes = root.path().join("recipes");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(&recipes).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "tests/fixtures/authored_root/editor_app_catalog/authored/apps/demo_app.yaml",
            ),
            apps.join("demo_app.yaml"),
        )
        .unwrap();
        fs::write(
            recipes.join("dependent.yaml"),
            "schema_version: 1\nkind: recipe\nid: dependent\nname: Dependent\nrecipe_dependencies: []\nprovides:\n  features: []\ninputs: {}\nartifacts: {}\nartifact_groups: {}\nsteps:\n  - id: launch\n    type: launch_app\n    name: Launch\n    user_toggleable: false\n    app_ref: demo_app\n    dependencies: []\n    constraints:\n      capabilities: []\n      conflicts_with: []\n    skip_if:\n      - type: unknown_condition\n        params:\n          path:\n            app_target: config_file\n    params: {}\n    verify: []\n",
        )
        .unwrap();
        let mut session = CatalogRefactorSession::with_storage(root.path().join("state"));
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameTarget {
                    app_id: "demo_app".into(),
                    target_id: "config_file".into(),
                    new_id: "config_file_new".into(),
                },
            )
            .unwrap();
        let error = session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Ok(()),
            )
            .unwrap_err();
        assert!(error.contains("app_target_inapplicable"));
        let app = fs::read_to_string(apps.join("demo_app.yaml")).unwrap();
        assert!(app.contains("config_file:"));
    }

    #[test]
    fn staged_overlay_ignores_unrelated_recipe_authority_errors() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        let recipes = root.path().join("recipes");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(&recipes).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "tests/fixtures/authored_root/editor_app_catalog/authored/apps/demo_app.yaml",
            ),
            apps.join("demo_app.yaml"),
        )
        .unwrap();
        fs::write(
            recipes.join("dependent.yaml"),
            r#"schema_version: 1
kind: recipe
id: dependent
name: Dependent
recipe_dependencies: []
provides:
  features: []
inputs: {}
artifacts: {}
artifact_groups: {}
steps:
  - id: target
    type: launch_app
    name: Target
    user_toggleable: false
    app_ref: demo_app
    dependencies: []
    constraints:
      capabilities: []
      conflicts_with: []
    skip_if: []
    params: {}
    verify:
      - type: path_exists
        params:
          path:
            app_target: config_file
  - id: unrelated
    type: launch_app
    name: Unrelated
    user_toggleable: false
    app_ref: missing_app
    dependencies: []
    constraints:
      capabilities: []
      conflicts_with: []
    skip_if: []
    params: {}
    verify: []
"#,
        )
        .unwrap();

        let mut session = CatalogRefactorSession::with_storage(root.path().join("state"));
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameTarget {
                    app_id: "demo_app".into(),
                    target_id: "config_file".into(),
                    new_id: "config_file_new".into(),
                },
            )
            .unwrap();
        let status = session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(status.state, CatalogChangeState::Completed);
        assert!(fs::read_to_string(recipes.join("dependent.yaml"))
            .unwrap()
            .contains("app_target: config_file_new"));
    }

    #[test]
    fn recovery_retry_rejects_dependencies_added_after_preview() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        let recipes = root.path().join("recipes");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(&recipes).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "tests/fixtures/authored_root/editor_app_catalog/authored/apps/demo_app.yaml",
            ),
            apps.join("demo_app.yaml"),
        )
        .unwrap();
        fs::write(
            recipes.join("dependent.yaml"),
            r#"schema_version: 1
kind: recipe
id: dependent
name: Dependent
recipe_dependencies: []
provides:
  features: []
inputs: {}
artifacts: {}
artifact_groups: {}
steps:
  - id: launch
    type: launch_app
    name: Launch
    user_toggleable: false
    app_ref: demo_app
    dependencies: []
    constraints:
      capabilities: []
      conflicts_with: []
    skip_if: []
    params: {}
    verify: []
"#,
        )
        .unwrap();

        let storage = root.path().join("state");
        let mut session = CatalogRefactorSession::with_storage(storage.clone());
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameApp {
                    app_id: "demo_app".into(),
                    new_id: "renamed_app".into(),
                },
            )
            .unwrap();
        let error = session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Err("session reconciliation failed".into()),
            )
            .unwrap_err();
        assert!(error.contains("session reconciliation"));

        fs::write(
            recipes.join("late.yaml"),
            r#"schema_version: 1
kind: recipe
id: late
name: Late
recipe_dependencies: []
provides:
  features: []
inputs: {}
artifacts: {}
artifact_groups: {}
steps:
  - id: launch
    type: launch_app
    name: Launch
    user_toggleable: false
    app_ref: demo_app
    dependencies: []
    constraints:
      capabilities: []
      conflicts_with: []
    skip_if: []
    params: {}
    verify: []
"#,
        )
        .unwrap();

        let restarted = CatalogRefactorSession::with_storage(storage);
        let error = restarted
            .retry_recovery(root.path(), &preview.operation_id, |_| Ok(()))
            .unwrap_err();
        assert!(error.contains("dependency closure changed"));
        assert!(error.contains("recipes/late.yaml"));
        assert!(fs::read_to_string(recipes.join("late.yaml"))
            .unwrap()
            .contains("app_ref: demo_app"));
    }

    #[test]
    fn completed_apply_replays_from_storage_after_relaunch_without_republishing() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        let recipes = root.path().join("recipes");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(&recipes).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "tests/fixtures/authored_root/editor_app_catalog/authored/apps/demo_app.yaml",
            ),
            apps.join("demo_app.yaml"),
        )
        .unwrap();
        fs::write(
            recipes.join("dependent.yaml"),
            r#"schema_version: 1
kind: recipe
id: dependent
name: Dependent
recipe_dependencies: []
provides:
  features: []
inputs: {}
artifacts: {}
artifact_groups: {}
steps:
  - id: launch
    type: launch_app
    name: Launch
    user_toggleable: false
    app_ref: demo_app
    dependencies: []
    constraints:
      capabilities: []
      conflicts_with: []
    skip_if: []
    params: {}
    verify: []
"#,
        )
        .unwrap();

        let storage = root.path().join("state");
        let mut session = CatalogRefactorSession::with_storage(storage.clone());
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameApp {
                    app_id: "demo_app".into(),
                    new_id: "renamed_app".into(),
                },
            )
            .unwrap();
        let completed = session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(completed.state, CatalogChangeState::Completed);

        let mut relaunched = CatalogRefactorSession::with_storage(storage);
        let replayed = relaunched
            .apply(
                &preview.operation_id,
                "preview-token-was-consumed",
                &[],
                &[],
                |_| panic!("completed apply must not reconcile or publish twice"),
            )
            .unwrap();
        assert_eq!(replayed.state, CatalogChangeState::Completed);
        assert_eq!(replayed.changed_files, completed.changed_files);
        assert!(fs::read_to_string(recipes.join("dependent.yaml"))
            .unwrap()
            .contains("app_ref: renamed_app"));
    }

    #[test]
    fn staged_overlay_rejects_competing_app_package_identity() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/authored_root/editor_app_catalog/authored/apps");
        fs::copy(fixtures.join("demo_app.yaml"), apps.join("demo_app.yaml")).unwrap();
        let competing = fs::read_to_string(fixtures.join("second_app.yaml"))
            .unwrap()
            .replace(
                "package_id: com.example.second",
                "package_id: com.example.demo",
            );
        fs::write(apps.join("second_app.yaml"), competing).unwrap();
        let mut session = CatalogRefactorSession::with_storage(root.path().join("state"));
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameTarget {
                    app_id: "demo_app".into(),
                    target_id: "config_file".into(),
                    new_id: "config_file_new".into(),
                },
            )
            .unwrap();

        let error = session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Ok(()),
            )
            .unwrap_err();

        assert!(error.contains("com.example.demo"));
        assert!(fs::read_to_string(apps.join("demo_app.yaml"))
            .unwrap()
            .contains("config_file:"));
    }

    #[cfg(unix)]
    #[test]
    fn app_identity_rename_preserves_source_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let source = apps.join("owner.yaml");
        fs::write(
            &source,
            include_str!("../../../authored/apps/armsx1.yaml").replace("id: armsx1", "id: owner"),
        )
        .unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        let mut session =
            CatalogRefactorSession::with_storage(root.path().join("application-state"));
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameApp {
                    app_id: "owner".into(),
                    new_id: "renamed".into(),
                },
            )
            .unwrap();

        let status = session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Ok(()),
            )
            .unwrap();

        assert_eq!(status.state, CatalogChangeState::Completed);
        assert_eq!(
            fs::metadata(apps.join("renamed.yaml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn preview_binds_exact_snapshot_and_consumes_its_token_once() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("apps")).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        fs::create_dir(root.path().join("device_profiles")).unwrap();
        fs::create_dir(root.path().join("device_plans")).unwrap();
        let app = AppDefinitionV1 {
            schema_version: SCHEMA_VERSION_V1,
            kind: APP_DEFINITION_KIND.into(),
            id: "old".into(),
            name: "Old".into(),
            description: None,
            category: None,
            package_id: "com.example.old".into(),
            artifacts: IndexMap::new(),
            permission_sets: None,
            targets: IndexMap::new(),
            launcher_activity: None,
            metadata: OrderedValueMap::new(),
        };
        fs::write(
            root.path().join("apps/old.yaml"),
            emit_app_definition_yaml(&app).unwrap(),
        )
        .unwrap();
        fs::write(
            root.path().join("recipes/example.yaml"),
            include_str!("../../../authored/recipes/feature.copy_bios.yaml")
                .replace("retroarch", "old"),
        )
        .unwrap();

        let mut session =
            CatalogRefactorSession::with_storage(root.path().join("application-state"));
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameApp {
                    app_id: "old".into(),
                    new_id: "new".into(),
                },
            )
            .unwrap();
        assert_eq!(
            preview.changed_files,
            vec![
                PathBuf::from("apps/new.yaml"),
                PathBuf::from("apps/old.yaml"),
                PathBuf::from("recipes/example.yaml")
            ]
        );
        assert_eq!(preview.references.len(), 2);
        assert!(session
            .take("different-operation", &preview.preview_token)
            .is_err());
        let prepared = session
            .take(&preview.operation_id, &preview.preview_token)
            .unwrap();
        assert_eq!(prepared.preview.snapshot_digest, preview.snapshot_digest);
        assert!(session
            .take(&preview.operation_id, &preview.preview_token)
            .is_err());
    }

    #[test]
    fn duplicate_apply_replays_completion_without_republishing() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("apps")).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let app = AppDefinitionV1 {
            schema_version: SCHEMA_VERSION_V1,
            kind: APP_DEFINITION_KIND.into(),
            id: "old".into(),
            name: "Old".into(),
            description: None,
            category: None,
            package_id: "com.example.old".into(),
            artifacts: IndexMap::new(),
            permission_sets: None,
            targets: IndexMap::new(),
            launcher_activity: None,
            metadata: OrderedValueMap::new(),
        };
        fs::write(
            root.path().join("apps/old.yaml"),
            emit_app_definition_yaml(&app).unwrap(),
        )
        .unwrap();
        let mut session =
            CatalogRefactorSession::with_storage(root.path().join("application-state"));
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameApp {
                    app_id: "old".into(),
                    new_id: "new".into(),
                },
            )
            .unwrap();
        let first = session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(first.state, CatalogChangeState::Completed);
        assert!(!root.path().join("apps/old.yaml").exists());
        let published = fs::read(root.path().join("apps/new.yaml")).unwrap();
        let second = session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(second.state, CatalogChangeState::Completed);
        assert_eq!(
            fs::read(root.path().join("apps/new.yaml")).unwrap(),
            published
        );
    }

    #[test]
    fn status_distinguishes_an_active_publication_from_recovery_after_lock_release() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("apps")).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        let record = OperationRecord::new("active-status", "catalog-refactor", "snapshot");
        coordinator.begin_operation(&guard, record).unwrap();
        coordinator
            .transition(&guard, "active-status", OperationPhase::Staged)
            .unwrap();
        coordinator
            .transition(&guard, "active-status", OperationPhase::Publishing)
            .unwrap();

        let session = CatalogRefactorSession::with_storage(storage.clone());
        assert_eq!(
            session
                .status(root.path(), "active-status")
                .unwrap()
                .unwrap()
                .state,
            CatalogChangeState::InProgress
        );
        drop(guard);
        assert_eq!(
            session
                .status(root.path(), "active-status")
                .unwrap()
                .unwrap()
                .state,
            CatalogChangeState::RecoveryRequired
        );
    }

    #[test]
    fn reconciliation_failure_remains_recoverable_and_children_cannot_retry_alone() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("apps")).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let app = AppDefinitionV1 {
            schema_version: SCHEMA_VERSION_V1,
            kind: APP_DEFINITION_KIND.into(),
            id: "old".into(),
            name: "Old".into(),
            description: None,
            category: None,
            package_id: "com.example.old".into(),
            artifacts: IndexMap::new(),
            permission_sets: None,
            targets: IndexMap::new(),
            launcher_activity: None,
            metadata: OrderedValueMap::new(),
        };
        fs::write(
            root.path().join("apps/old.yaml"),
            emit_app_definition_yaml(&app).unwrap(),
        )
        .unwrap();
        let storage = root.path().join("application-state");
        let mut session = CatalogRefactorSession::with_storage(storage.clone());
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameApp {
                    app_id: "old".into(),
                    new_id: "new".into(),
                },
            )
            .unwrap();
        assert!(session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Err("injected reload failure".into()),
            )
            .is_err());
        assert!(root.path().join("apps/new.yaml").exists());
        let recoveries = session.list_recoveries(root.path()).unwrap();
        assert_eq!(recoveries.len(), 1);
        assert_eq!(recoveries[0].operation_id, preview.operation_id);
        assert_eq!(recoveries[0].state, CatalogChangeState::RecoveryRequired);

        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let parent = coordinator
            .operation(&preview.operation_id)
            .unwrap()
            .unwrap();
        assert_eq!(parent.child_operation_ids.len(), 1);
        assert!(session
            .retry_recovery(root.path(), &parent.child_operation_ids[0], |_| Ok(()))
            .is_err());
        let completed = session
            .retry_recovery(root.path(), &preview.operation_id, |_| Ok(()))
            .unwrap();
        assert_eq!(completed.state, CatalogChangeState::Completed);
    }

    #[test]
    fn relaunch_retries_a_predeclared_child_after_the_parent_update_window() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let destination = apps.join("demo.yaml");
        fs::write(&destination, b"old").unwrap();
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        let child_id = coordinator
            .child_write_operation_id("group-stage-crash", &destination)
            .unwrap();
        let mut parent = OperationRecord::new("group-stage-crash", "catalog-refactor", "request");
        parent.child_operation_ids.push(child_id.clone());
        parent.changed_files.push(PathBuf::from("apps/demo.yaml"));
        coordinator.begin_operation(&guard, parent).unwrap();
        let expected = coordinator.snapshot_file(&destination).unwrap();
        coordinator
            .stage_file_write_for_parent(
                &guard,
                "group-stage-crash",
                &destination,
                expected.as_ref(),
                b"new",
            )
            .unwrap();
        drop(guard);
        drop(coordinator);

        let relaunched = CatalogRefactorSession::with_storage(storage.clone());
        let recoveries = relaunched.list_recoveries(root.path()).unwrap();
        assert_eq!(recoveries.len(), 1);
        assert_eq!(recoveries[0].operation_id, "group-stage-crash");
        let completed = relaunched
            .retry_recovery(root.path(), "group-stage-crash", |_| Ok(()))
            .unwrap();
        assert_eq!(completed.state, CatalogChangeState::Completed);
        assert_eq!(fs::read(destination).unwrap(), b"new");
    }

    #[test]
    fn changed_catalog_inventory_rejects_a_new_dependent_recipe_after_preview() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("apps")).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let app = AppDefinitionV1 {
            schema_version: SCHEMA_VERSION_V1,
            kind: APP_DEFINITION_KIND.into(),
            id: "old".into(),
            name: "Old".into(),
            description: None,
            category: None,
            package_id: "com.example.old".into(),
            artifacts: IndexMap::new(),
            permission_sets: None,
            targets: IndexMap::new(),
            launcher_activity: None,
            metadata: OrderedValueMap::new(),
        };
        let app_path = root.path().join("apps/old.yaml");
        fs::write(&app_path, emit_app_definition_yaml(&app).unwrap()).unwrap();
        let storage = root.path().join("application-state");
        let mut session = CatalogRefactorSession::with_storage(storage);
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameApp {
                    app_id: "old".into(),
                    new_id: "new".into(),
                },
            )
            .unwrap();
        fs::write(
            root.path().join("recipes/late-dependent.yaml"),
            include_str!("../../../authored/recipes/feature.copy_bios.yaml")
                .replace("retroarch", "old"),
        )
        .unwrap();

        assert!(session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Ok(())
            )
            .is_err());
        assert!(app_path.exists());
        assert!(!root.path().join("apps/new.yaml").exists());
        let status = session
            .status(root.path(), &preview.operation_id)
            .unwrap()
            .unwrap();
        assert_eq!(status.state, CatalogChangeState::Rejected);
    }

    #[test]
    fn missing_child_before_publication_is_durably_rejected() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let destination = apps.join("demo.yaml");
        fs::write(&destination, b"old").unwrap();
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        let child_id = coordinator
            .child_write_operation_id("group-missing-child", &destination)
            .unwrap();
        let mut parent = OperationRecord::new("group-missing-child", "catalog-refactor", "request");
        parent.child_operation_ids.push(child_id);
        parent.changed_files.push(PathBuf::from("apps/demo.yaml"));
        coordinator.begin_operation(&guard, parent).unwrap();
        drop(guard);

        let relaunched = CatalogRefactorSession::with_storage(storage);
        let result = relaunched
            .retry_recovery(root.path(), "group-missing-child", |_| Ok(()))
            .unwrap();
        assert_eq!(result.state, CatalogChangeState::Rejected);
        assert_eq!(fs::read(destination).unwrap(), b"old");
    }

    #[test]
    fn grouped_recovery_is_discovered_and_completed_after_full_session_relaunch() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let destination = apps.join("demo.yaml");
        fs::write(&destination, b"old").unwrap();
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        let mut parent = OperationRecord::new("group-relaunch", "catalog-refactor", "request");
        parent.changed_files = vec![PathBuf::from("apps/demo.yaml")];
        coordinator.begin_operation(&guard, parent.clone()).unwrap();
        let child = coordinator
            .stage_file_write(&guard, &destination, b"new")
            .unwrap();
        parent.child_operation_ids.push(child);
        coordinator.update_operation(&guard, parent).unwrap();
        coordinator
            .transition(&guard, "group-relaunch", OperationPhase::Staged)
            .unwrap();
        coordinator
            .transition(&guard, "group-relaunch", OperationPhase::Publishing)
            .unwrap();
        drop(guard);
        drop(coordinator);

        let relaunched = CatalogRefactorSession::with_storage(storage);
        let recoveries = relaunched.list_recoveries(root.path()).unwrap();
        assert!(recoveries
            .iter()
            .any(|status| status.operation_id == "group-relaunch"));
        assert_eq!(
            recoveries
                .iter()
                .find(|status| status.operation_id == "group-relaunch")
                .unwrap()
                .changed_files,
            vec![PathBuf::from("apps/demo.yaml")]
        );
        let completed = relaunched
            .retry_recovery(root.path(), "group-relaunch", |_| Ok(()))
            .unwrap();
        assert_eq!(completed.state, CatalogChangeState::Completed);
        assert_eq!(fs::read(destination).unwrap(), b"new");

        let replay = relaunched
            .status(root.path(), "group-relaunch")
            .unwrap()
            .unwrap();
        assert_eq!(replay, completed);
    }

    #[test]
    fn grouped_retry_rolls_back_owned_files_and_preserves_external_edits() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let first = apps.join("first.yaml");
        let second = apps.join("second.yaml");
        fs::write(&first, b"first-old").unwrap();
        fs::write(&second, b"second-old").unwrap();
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        let mut parent = OperationRecord::new("group-conflict", "catalog-refactor", "request");
        parent.changed_files = vec![
            PathBuf::from("apps/first.yaml"),
            PathBuf::from("apps/second.yaml"),
        ];
        coordinator.begin_operation(&guard, parent.clone()).unwrap();
        let first_child = coordinator
            .stage_file_write(&guard, &first, b"first-new")
            .unwrap();
        let second_child = coordinator
            .stage_file_write(&guard, &second, b"second-new")
            .unwrap();
        parent.child_operation_ids = vec![first_child.clone(), second_child];
        coordinator.update_operation(&guard, parent).unwrap();
        coordinator
            .transition(&guard, "group-conflict", OperationPhase::Staged)
            .unwrap();
        coordinator
            .transition(&guard, "group-conflict", OperationPhase::Publishing)
            .unwrap();
        coordinator
            .publish_staged_file_write(&guard, &first_child)
            .unwrap();
        coordinator
            .transition(&guard, "group-conflict", OperationPhase::RecoveryRequired)
            .unwrap();
        fs::write(&second, b"external-edit").unwrap();
        drop(guard);
        drop(coordinator);

        let relaunched = CatalogRefactorSession::with_storage(storage);
        assert!(relaunched
            .retry_recovery(root.path(), "group-conflict", |_| Ok(()))
            .is_err());
        assert_eq!(fs::read(first).unwrap(), b"first-old");
        assert_eq!(fs::read(second).unwrap(), b"external-edit");
        assert_eq!(
            relaunched
                .status(root.path(), "group-conflict")
                .unwrap()
                .unwrap()
                .state,
            CatalogChangeState::RecoveryRequired
        );
    }

    #[test]
    fn grouped_retry_republishes_children_restored_by_prior_rollback() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let first = apps.join("first.yaml");
        let second = apps.join("second.yaml");
        fs::write(&first, b"first-old").unwrap();
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        let mut parent = OperationRecord::new("group-republish", "catalog-refactor", "request");
        parent.changed_files = vec![
            PathBuf::from("apps/first.yaml"),
            PathBuf::from("apps/second.yaml"),
        ];
        coordinator.begin_operation(&guard, parent.clone()).unwrap();
        let first_child = coordinator
            .stage_file_write(&guard, &first, b"first-new")
            .unwrap();
        let second_child = coordinator
            .stage_file_write(&guard, &second, b"second-new")
            .unwrap();
        parent.child_operation_ids = vec![first_child.clone(), second_child];
        coordinator.update_operation(&guard, parent).unwrap();
        coordinator
            .transition(&guard, "group-republish", OperationPhase::Staged)
            .unwrap();
        coordinator
            .transition(&guard, "group-republish", OperationPhase::Publishing)
            .unwrap();
        coordinator
            .publish_staged_file_write_until_reconciled(&guard, &first_child)
            .unwrap();
        coordinator
            .transition(&guard, "group-republish", OperationPhase::RecoveryRequired)
            .unwrap();
        fs::write(&second, b"external edit").unwrap();
        drop(guard);
        drop(coordinator);

        let session = CatalogRefactorSession::with_storage(storage.clone());
        assert!(session
            .retry_recovery(root.path(), "group-republish", |_| Ok(()))
            .is_err());
        assert_eq!(fs::read(&first).unwrap(), b"first-old");
        assert_eq!(fs::read(&second).unwrap(), b"external edit");

        fs::remove_file(&second).unwrap();
        let completed = session
            .retry_recovery(root.path(), "group-republish", |_| Ok(()))
            .unwrap();
        assert_eq!(completed.state, CatalogChangeState::Completed);
        assert_eq!(fs::read(&first).unwrap(), b"first-new");
        assert_eq!(fs::read(&second).unwrap(), b"second-new");
    }

    #[test]
    fn interrupted_intermediate_phase_is_reported_as_recovery_required() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("apps")).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        coordinator
            .begin_operation(
                &guard,
                OperationRecord::new("interrupted-status", "catalog-refactor", "request"),
            )
            .unwrap();
        coordinator
            .transition(&guard, "interrupted-status", OperationPhase::Staged)
            .unwrap();
        drop(guard);
        drop(coordinator);

        let status = CatalogRefactorSession::with_storage(storage)
            .status(root.path(), "interrupted-status")
            .unwrap()
            .unwrap();

        assert_eq!(status.state, CatalogChangeState::RecoveryRequired);
    }

    #[test]
    fn standalone_recovery_waits_for_session_reconciliation_before_completion() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let destination = apps.join("demo.yaml");
        fs::write(&destination, b"original").unwrap();
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        let operation_id = coordinator
            .stage_file_write(&guard, &destination, b"recovered")
            .unwrap();
        drop(guard);
        drop(coordinator);

        let session = CatalogRefactorSession::with_storage(storage);
        assert!(session
            .retry_recovery(root.path(), &operation_id, |_| {
                Err("session reload failed".into())
            })
            .is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"recovered");
        assert_eq!(
            session
                .status(root.path(), &operation_id)
                .unwrap()
                .unwrap()
                .state,
            CatalogChangeState::RecoveryRequired
        );
    }

    #[test]
    fn interrupted_group_publication_reports_recovery_required_after_external_edit() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let destination = apps.join("demo.yaml");
        fs::write(&destination, b"original").unwrap();
        let storage = root.path().join("application-state");
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(root.path(), &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        let mut parent = OperationRecord::new(
            "group-interrupted-publication",
            "catalog-refactor",
            "request",
        );
        parent.changed_files.push(PathBuf::from("apps/demo.yaml"));
        coordinator.begin_operation(&guard, parent.clone()).unwrap();
        let child = coordinator
            .stage_file_write(&guard, &destination, b"staged")
            .unwrap();
        parent.child_operation_ids.push(child);
        coordinator.update_operation(&guard, parent).unwrap();
        coordinator
            .transition(
                &guard,
                "group-interrupted-publication",
                OperationPhase::Staged,
            )
            .unwrap();
        coordinator
            .transition(
                &guard,
                "group-interrupted-publication",
                OperationPhase::Publishing,
            )
            .unwrap();
        fs::write(&destination, b"external edit").unwrap();
        drop(guard);
        drop(coordinator);

        let relaunched = CatalogRefactorSession::with_storage(storage);
        assert!(relaunched
            .retry_recovery(root.path(), "group-interrupted-publication", |_| Ok(()))
            .is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"external edit");
        assert_eq!(
            relaunched
                .status(root.path(), "group-interrupted-publication")
                .unwrap()
                .unwrap()
                .state,
            CatalogChangeState::RecoveryRequired
        );
    }

    #[test]
    fn whole_app_delete_blocks_typed_dependencies_and_never_cascades_recipes() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("apps")).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let app = AppDefinitionV1 {
            schema_version: SCHEMA_VERSION_V1,
            kind: APP_DEFINITION_KIND.into(),
            id: "old".into(),
            name: "Old".into(),
            description: None,
            category: None,
            package_id: "com.example.old".into(),
            artifacts: IndexMap::new(),
            permission_sets: None,
            targets: IndexMap::new(),
            launcher_activity: None,
            metadata: OrderedValueMap::new(),
        };
        fs::write(
            root.path().join("apps/old.yaml"),
            emit_app_definition_yaml(&app).unwrap(),
        )
        .unwrap();
        let recipe_path = root.path().join("recipes/dependent.yaml");
        fs::write(
            &recipe_path,
            include_str!("../../../authored/recipes/feature.copy_bios.yaml")
                .replace("retroarch", "old"),
        )
        .unwrap();
        let storage = root.path().join("application-state");
        let mut session = CatalogRefactorSession::with_storage(storage);
        let error = session
            .preview(
                root.path(),
                CatalogChangeIntent::DeleteApp {
                    app_id: "old".into(),
                },
            )
            .unwrap_err();
        assert!(error.contains("referenced"));
        assert!(error.contains("recipes/dependent.yaml"));
        assert!(error.contains("steps.copy_bios_dir"), "{error}");
        assert!(error.contains("app_ref"));
        fs::remove_file(&recipe_path).unwrap();
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::DeleteApp {
                    app_id: "old".into(),
                },
            )
            .unwrap();
        let status = session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(status.state, CatalogChangeState::Completed);
        assert!(!root.path().join("apps/old.yaml").exists());
        assert!(root.path().join("recipes").is_dir());
    }

    #[test]
    fn recovery_after_app_unlink_rolls_forward_without_restoring_published_children() {
        let root = tempfile::tempdir().unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let apps = canonical_root.join("apps");
        fs::create_dir(&apps).unwrap();
        fs::create_dir(canonical_root.join("recipes")).unwrap();
        let source = apps.join("armsx1.yaml");
        fs::write(&source, include_str!("../../../authored/apps/armsx1.yaml")).unwrap();
        let storage = root.path().join("application-state");
        let mut session = CatalogRefactorSession::with_storage(storage.clone());
        let preview = session
            .preview(
                &canonical_root,
                CatalogChangeIntent::DeleteApp {
                    app_id: "armsx1".into(),
                },
            )
            .unwrap();
        let prepared = session.previews.get(&preview.preview_token).unwrap();
        let (_, source_snapshot) = prepared.delete_source.as_ref().unwrap();
        let coordinator =
            CatalogChangeCoordinator::open_with_storage(&canonical_root, &storage).unwrap();
        let guard = coordinator.acquire_publication().unwrap();
        let mut parent = OperationRecord::new(
            &preview.operation_id,
            "catalog-refactor",
            &preview.snapshot_digest,
        );
        parent.snapshot_digest = Some(preview.snapshot_digest.clone());
        parent.changed_files = preview.changed_files.clone();
        parent.delete_source = Some(DurableSourceDeletion {
            source: PathBuf::from("apps/armsx1.yaml"),
            expected_digest: digest(&source_snapshot.bytes),
            expected_identity: Some(source_snapshot.identity.clone()),
        });
        coordinator.begin_operation(&guard, parent.clone()).unwrap();
        let destination = apps.join("replacement.yaml");
        let child = coordinator
            .stage_file_write_for_parent(
                &guard,
                &preview.operation_id,
                &destination,
                None,
                b"published replacement",
            )
            .unwrap();
        parent.child_operation_ids.push(child.clone());
        coordinator.update_operation(&guard, parent).unwrap();
        coordinator
            .transition(&guard, &preview.operation_id, OperationPhase::Staged)
            .unwrap();
        coordinator
            .transition(&guard, &preview.operation_id, OperationPhase::Publishing)
            .unwrap();
        coordinator
            .publish_staged_file_write_until_reconciled(&guard, &child)
            .unwrap();
        fs::remove_file(&source).unwrap();
        coordinator
            .transition(
                &guard,
                &preview.operation_id,
                OperationPhase::RecoveryRequired,
            )
            .unwrap();
        drop(guard);
        drop(coordinator);

        let relaunched = CatalogRefactorSession::with_storage(storage);
        let status = relaunched
            .retry_recovery(&canonical_root, &preview.operation_id, |_| Ok(()))
            .unwrap();

        assert_eq!(status.state, CatalogChangeState::Completed);
        assert!(!source.exists());
        assert_eq!(fs::read(destination).unwrap(), b"published replacement");
    }

    #[test]
    fn dirty_rejection_reports_every_affected_open_path_and_is_durable() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("apps")).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let app = AppDefinitionV1 {
            schema_version: SCHEMA_VERSION_V1,
            kind: APP_DEFINITION_KIND.into(),
            id: "old".into(),
            name: "Old".into(),
            description: None,
            category: None,
            package_id: "com.example.old".into(),
            artifacts: IndexMap::new(),
            permission_sets: None,
            targets: IndexMap::new(),
            launcher_activity: None,
            metadata: OrderedValueMap::new(),
        };
        let app_path = root.path().join("apps/old.yaml");
        let recipe_path = root.path().join("recipes/dependent.yaml");
        fs::write(&app_path, emit_app_definition_yaml(&app).unwrap()).unwrap();
        fs::write(
            &recipe_path,
            include_str!("../../../authored/recipes/feature.copy_bios.yaml")
                .replace("retroarch", "old"),
        )
        .unwrap();
        let storage = root.path().join("application-state");
        let mut session = CatalogRefactorSession::with_storage(storage.clone());
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameApp {
                    app_id: "old".into(),
                    new_id: "new".into(),
                },
            )
            .unwrap();
        let error = session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[recipe_path.clone(), app_path.clone()],
                &[],
                |_| Ok(()),
            )
            .unwrap_err();
        assert!(error.contains("apps/old.yaml"));
        assert!(error.contains("recipes/dependent.yaml"));
        let relaunched = CatalogRefactorSession::with_storage(storage);
        let status = relaunched
            .status(root.path(), &preview.operation_id)
            .unwrap()
            .unwrap();
        assert_eq!(status.state, CatalogChangeState::Rejected);
        assert_eq!(status.changed_files, preview.changed_files);
        assert!(app_path.exists());
        assert!(!root.path().join("apps/new.yaml").exists());
    }

    #[test]
    fn final_snapshot_check_rejects_external_edits_before_staging() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("apps")).unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        let app = AppDefinitionV1 {
            schema_version: SCHEMA_VERSION_V1,
            kind: APP_DEFINITION_KIND.into(),
            id: "old".into(),
            name: "Old".into(),
            description: None,
            category: None,
            package_id: "com.example.old".into(),
            artifacts: IndexMap::new(),
            permission_sets: None,
            targets: IndexMap::new(),
            launcher_activity: None,
            metadata: OrderedValueMap::new(),
        };
        let app_path = root.path().join("apps/old.yaml");
        fs::write(&app_path, emit_app_definition_yaml(&app).unwrap()).unwrap();
        let storage = root.path().join("application-state");
        let mut session = CatalogRefactorSession::with_storage(storage.clone());
        let preview = session
            .preview(
                root.path(),
                CatalogChangeIntent::RenameApp {
                    app_id: "old".into(),
                    new_id: "new".into(),
                },
            )
            .unwrap();
        fs::write(&app_path, b"external edit").unwrap();
        assert!(session
            .apply(
                &preview.operation_id,
                &preview.preview_token,
                &[],
                &[],
                |_| Ok(())
            )
            .is_err());
        assert_eq!(fs::read(&app_path).unwrap(), b"external edit");
        assert!(!root.path().join("apps/new.yaml").exists());
        let relaunched = CatalogRefactorSession::with_storage(storage);
        assert_eq!(
            relaunched
                .status(root.path(), &preview.operation_id)
                .unwrap()
                .unwrap()
                .state,
            CatalogChangeState::Rejected
        );
    }

    fn recipe() -> Recipe {
        let mut artifacts = OrderedMap::new();
        artifacts.insert(
            "local".into(),
            RecipeArtifact::AppArtifact(AppArtifactReference {
                app_ref: "old".into(),
                artifact: "apk".into(),
                input: None,
                cache: "default".into(),
            }),
        );
        let mut params = OrderedMap::new();
        params.insert("destination".into(), ParamValue::AppTarget("config".into()));
        let condition = StepCondition {
            type_name: "path_exists".into(),
            params: OrderedMap::from([("path".into(), json!({ "app_target": "config" }))]),
        };
        Recipe {
            id: "example".into(),
            kind: "recipe".into(),
            name: "Example".into(),
            description: None,
            schema_version: 1,
            recipe_dependencies: Vec::new(),
            provides: RecipeProvides {
                features: Vec::new(),
            },
            inputs: OrderedMap::new(),
            artifacts,
            artifact_groups: OrderedMap::new(),
            steps: vec![Step {
                id: "step".into(),
                type_name: "copy_file".into(),
                name: "old".into(),
                description: None,
                progress_note: None,
                user_toggleable: false,
                app_ref: Some("old".into()),
                dependencies: Vec::new(),
                constraints: StepConstraints {
                    capabilities: Vec::new(),
                    conflicts_with: Vec::new(),
                },
                skip_if: vec![condition.clone()],
                params,
                verify: vec![condition],
            }],
        }
    }
}
