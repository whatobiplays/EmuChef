//! Combined App/Recipe publication tests for the generated App pair service.
//!
//! These tests exercise the bounded publisher directly: the Recipe commit is
//! the first filesystem change and the App commit is the final one, so a
//! refusal discovered after the Recipe exists must either remove that Recipe
//! (when its identity still matches) or report that recovery is required.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::app_definition_admission::{
    admit_generated_app_pair_save, GeneratedAppPairSnapshot, GeneratedAppSaveContext,
};
use crate::app_definition_persistence::{
    capture_opened_app_definition_file, persist_generated_app_pair, persist_generated_app_pair_with,
    GeneratedPairPersistFailure, OpenedAppDefinitionFile,
};
use crate::authored_models::{self, AppDefinitionV1};
use crate::generation::{generate_app_recipe_draft, AppRecipeDraftRequest};
use crate::model::Recipe;
use crate::yaml;

struct PairFixture {
    app: AppDefinitionV1,
    recipe: Recipe,
    recipe_canonical_yaml: String,
    app_destination: PathBuf,
    recipe_destination: PathBuf,
}

impl PairFixture {
    fn context(
        &self,
        root: &Path,
        opened_app: Option<OpenedAppDefinitionFile>,
    ) -> GeneratedAppSaveContext {
        GeneratedAppSaveContext {
            authored_root: root.to_path_buf(),
            app_destination: self.app_destination.clone(),
            opened_app,
            recipe_destination: self.recipe_destination.clone(),
        }
    }
}

/// Build one generated App/Recipe pair for a catalog root without writing.
fn pair_fixture(root: &Path, package: &str) -> PairFixture {
    let request: AppRecipeDraftRequest = serde_json::from_value(serde_json::json!({
        "facts": {
            "packageName": package,
            "applicationLabel": "Example Player",
            "launcherActivities": [format!("{package}/.MainActivity")],
            "calculatedSha256": "b".repeat(64),
            "checksumStatus": "not_compared",
            "signatureVerification": "not_performed",
            "split": false,
            "base": true,
        },
        "appArtifactId": "apk",
    }))
    .expect("generation request deserializes");
    let draft = generate_app_recipe_draft(request);
    assert!(!draft.blocking, "{:?}", draft.diagnostics);
    let recipe_canonical_yaml = draft
        .recipe_canonical_yaml
        .clone()
        .expect("generated Recipe YAML");
    let recipe_relative = draft
        .recipe_destination
        .relative_path
        .clone()
        .expect("Recipe destination");
    let mapping: serde_yaml::Mapping =
        serde_yaml::from_str(&recipe_canonical_yaml).expect("Recipe YAML parses");
    let recipe = yaml::parse_recipe_mapping(&mapping, Path::new(&recipe_relative))
        .expect("Recipe mapping is valid");
    let app_relative = draft
        .app_destination
        .relative_path
        .clone()
        .expect("App destination");
    PairFixture {
        app: draft.app,
        recipe,
        recipe_canonical_yaml,
        app_destination: root.join(app_relative),
        recipe_destination: root.join(recipe_relative),
    }
}

fn admit(
    fixture: &PairFixture,
    root: &Path,
    opened_app: Option<OpenedAppDefinitionFile>,
) -> GeneratedAppPairSnapshot {
    let admission = admit_generated_app_pair_save(
        &fixture.app,
        &fixture.recipe,
        &fixture.recipe_canonical_yaml,
        &fixture.context(root, opened_app),
    );
    assert!(admission.can_save(), "{:?}", admission.diagnostics);
    admission.snapshot.expect("admitted pair snapshot")
}

fn catalog() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("temporary catalog root");
    fs::create_dir(root.path().join("apps")).unwrap();
    fs::create_dir(root.path().join("recipes")).unwrap();
    root
}

#[test]
fn generated_pair_publication_creates_the_recipe_then_the_app() {
    let catalog = catalog();
    let root = catalog.path().canonicalize().unwrap();
    let fixture = pair_fixture(root.as_path(), "com.example.player");
    let snapshot = admit(&fixture, root.as_path(), None);

    let outcome = persist_generated_app_pair(&snapshot).expect("publication succeeds");
    assert!(!outcome.app_replaced);
    assert_eq!(outcome.app.path, fixture.app_destination);
    assert_eq!(outcome.recipe_path, fixture.recipe_destination);
    let recipe_text = fs::read_to_string(&fixture.recipe_destination).unwrap();
    assert_eq!(recipe_text, fixture.recipe_canonical_yaml);
    let app_text = fs::read_to_string(&fixture.app_destination).unwrap();
    assert!(app_text.contains("com.example.player"), "{app_text}");
    assert!(app_text.contains("apk"), "{app_text}");
}

#[test]
fn generated_pair_publication_replaces_an_opened_app_and_keeps_its_file_mode() {
    let catalog = catalog();
    let root = catalog.path().canonicalize().unwrap();
    let fixture = pair_fixture(root.as_path(), "com.example.player");
    let baseline = pair_fixture(root.as_path(), "com.example.player");
    let baseline_text = authored_models::emit_app_definition_yaml(&baseline.app).unwrap();
    fs::write(&fixture.app_destination, baseline_text).unwrap();
    let mut permissions = fs::metadata(&fixture.app_destination)
        .unwrap()
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&fixture.app_destination, permissions).unwrap();
    let opened = capture_opened_app_definition_file(&fixture.app_destination).unwrap();

    let snapshot = admit(&fixture, root.as_path(), Some(opened));
    let outcome = persist_generated_app_pair(&snapshot).expect("publication succeeds");
    assert!(outcome.app_replaced);
    let mode = fs::metadata(&fixture.app_destination)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "the replaced App keeps its Unix mode");
}

#[test]
fn generated_pair_publication_removes_its_new_recipe_when_the_app_commit_is_stale() {
    let catalog = catalog();
    let root = catalog.path().canonicalize().unwrap();
    let fixture = pair_fixture(root.as_path(), "com.example.player");
    let snapshot = admit(&fixture, root.as_path(), None);
    let app_destination = fixture.app_destination.clone();
    let recipe_destination = fixture.recipe_destination.clone();

    let failure = persist_generated_app_pair_with(&snapshot, || {
        // An external writer claims the App destination between the Recipe
        // commit and the final App commit.
        fs::write(&app_destination, "external writer\n").unwrap();
    })
    .expect_err("the stale App destination must refuse publication");

    let GeneratedPairPersistFailure::RecipeCommitted { rollback, .. } = failure else {
        panic!("expected the Recipe-first failure shape: {failure:?}");
    };
    assert!(rollback.recipe_removed, "{rollback:?}");
    assert!(!rollback.recipe_recovery_required, "{rollback:?}");
    assert!(!recipe_destination.exists(), "the owned Recipe is removed");
    assert_eq!(
        fs::read_to_string(&app_destination).unwrap(),
        "external writer\n",
        "the external file is never overwritten"
    );
}

#[test]
fn generated_pair_publication_reports_recovery_when_the_committed_recipe_changed() {
    let catalog = catalog();
    let root = catalog.path().canonicalize().unwrap();
    let fixture = pair_fixture(root.as_path(), "com.example.player");
    let snapshot = admit(&fixture, root.as_path(), None);
    let app_destination = fixture.app_destination.clone();
    let recipe_destination = fixture.recipe_destination.clone();

    let failure = persist_generated_app_pair_with(&snapshot, || {
        // The external writer claims both destinations: the App destination
        // refuses the final commit and the Recipe no longer matches the exact
        // identity this publication created.
        fs::write(&app_destination, "external writer\n").unwrap();
        fs::write(&recipe_destination, "rewritten externally\n").unwrap();
    })
    .expect_err("the stale App destination must refuse publication");

    let GeneratedPairPersistFailure::RecipeCommitted { rollback, .. } = failure else {
        panic!("expected the Recipe-first failure shape: {failure:?}");
    };
    assert!(!rollback.recipe_removed, "{rollback:?}");
    assert!(rollback.recipe_recovery_required, "{rollback:?}");
    assert_eq!(
        fs::read_to_string(&recipe_destination).unwrap(),
        "rewritten externally\n",
        "a Recipe this publication no longer owns is preserved"
    );
}

#[test]
fn generated_pair_publication_refuses_a_recipe_destination_that_already_exists() {
    let catalog = catalog();
    let root = catalog.path().canonicalize().unwrap();
    let fixture = pair_fixture(root.as_path(), "com.example.player");
    let snapshot = admit(&fixture, root.as_path(), None);

    // An external writer claims the Recipe destination after admission but
    // before the commit, so the no-clobber Recipe commit must refuse first.
    fs::write(&fixture.recipe_destination, "existing recipe\n").unwrap();

    let failure = persist_generated_app_pair(&snapshot)
        .expect_err("the Recipe destination must never be clobbered");
    assert!(
        matches!(failure, GeneratedPairPersistFailure::Refused(_)),
        "{failure:?}"
    );
    assert!(!fixture.app_destination.exists(), "no App is written");
    assert_eq!(
        fs::read_to_string(&fixture.recipe_destination).unwrap(),
        "existing recipe\n"
    );

    // A later admission of the same pair refuses the occupied destination
    // before any publication capture exists.
    let admission = admit_generated_app_pair_save(
        &fixture.app,
        &fixture.recipe,
        &fixture.recipe_canonical_yaml,
        &fixture.context(root.as_path(), None),
    );
    assert!(!admission.can_save(), "{:?}", admission.diagnostics);
    assert!(admission.snapshot.is_none());
}

/// Write one persisted Recipe that references the fixture App so admission
/// captures it as an affected dependent Recipe.
///
/// The persisted Recipe is derived from the canonical generated Recipe so it
/// stays valid under the same authored contract, with only its identity
/// changed to reserve the generated destination for the new pair.
fn write_affected_recipe(root: &Path, fixture: &PairFixture) -> PathBuf {
    let canonical = fixture
        .recipe_canonical_yaml
        .replace("app.example_player.install", "app.example_player.extra_install")
        .replace("name: Install Example Player", "name: Existing Extra Install");
    let path = root.join("recipes/app.example_player.extra_install.yaml");
    fs::write(&path, canonical).unwrap();
    path
}

/// Write the fixture App into the catalog and capture it as an opened file.
fn write_and_open_app(fixture: &PairFixture) -> OpenedAppDefinitionFile {
    let text = authored_models::emit_app_definition_yaml(&fixture.app).unwrap();
    fs::write(&fixture.app_destination, text).unwrap();
    capture_opened_app_definition_file(&fixture.app_destination).unwrap()
}

#[test]
fn generated_pair_publication_refuses_a_late_external_edit_to_the_opened_app() {
    let catalog = catalog();
    let root = catalog.path().canonicalize().unwrap();
    let fixture = pair_fixture(root.as_path(), "com.example.player");
    let opened = write_and_open_app(&fixture);
    let snapshot = admit(&fixture, root.as_path(), Some(opened));
    let app_destination = fixture.app_destination.clone();
    let recipe_destination = fixture.recipe_destination.clone();
    let edit = "the external writer replaced the App text\n".to_string();
    let edited = edit.clone();

    let failure = persist_generated_app_pair_with(&snapshot, || {
        // The external writer edits the opened App after the Recipe commit.
        fs::write(&app_destination, &edited).unwrap();
    })
    .expect_err("the late App edit must refuse the final commit");

    match failure {
        GeneratedPairPersistFailure::RecipeCommitted { error, .. } => {
            assert_eq!(error.code, "app_definition_stale_state", "{error:?}");
        }
        other => panic!("expected the Recipe-first failure shape: {other:?}"),
    }
    assert!(!recipe_destination.exists(), "the owned Recipe is removed");
    assert_eq!(
        fs::read_to_string(&app_destination).unwrap(),
        edit,
        "the external edit is never overwritten",
    );
}

#[test]
fn generated_pair_publication_refuses_a_late_edit_to_an_affected_persisted_recipe() {
    let catalog = catalog();
    let root = catalog.path().canonicalize().unwrap();
    let fixture = pair_fixture(root.as_path(), "com.example.player");
    let opened = write_and_open_app(&fixture);
    let affected_recipe = write_affected_recipe(root.as_path(), &fixture);
    let snapshot = admit(&fixture, root.as_path(), Some(opened));
    let app_destination = fixture.app_destination.clone();
    let recipe_destination = fixture.recipe_destination.clone();
    let edit = "the external writer replaced the dependent Recipe text\n".to_string();
    let edited = edit.clone();

    let failure = persist_generated_app_pair_with(&snapshot, || {
        // The external writer edits the captured dependent Recipe after the
        // Recipe commit and before the final App commit.
        fs::write(&affected_recipe, &edited).unwrap();
    })
    .expect_err("the late affected-Recipe edit must refuse the final commit");

    match failure {
        GeneratedPairPersistFailure::RecipeCommitted { error, .. } => {
            assert_eq!(error.code, "app_definition_stale_state", "{error:?}");
        }
        other => panic!("expected the Recipe-first failure shape: {other:?}"),
    }
    assert!(!recipe_destination.exists(), "the owned Recipe is removed");
    assert_eq!(
        fs::read_to_string(&app_destination).unwrap(),
        authored_models::emit_app_definition_yaml(&fixture.app).unwrap(),
        "the App is never replaced after the dependent Recipe changed",
    );
    assert_eq!(
        fs::read_to_string(&affected_recipe).unwrap(),
        edit,
        "the external dependent-Recipe edit is preserved",
    );
}

#[test]
fn generated_pair_publication_reports_recovery_when_the_recipe_inode_changed_under_identical_bytes() {
    let catalog = catalog();
    let root = catalog.path().canonicalize().unwrap();
    let fixture = pair_fixture(root.as_path(), "com.example.player");
    let snapshot = admit(&fixture, root.as_path(), None);
    let recipe_destination = fixture.recipe_destination.clone();
    let replacement_source = root.join("replacement-recipe.yaml");
    let mut observed = None;

    let failure = persist_generated_app_pair_with(&snapshot, || {
        // The owned Recipe exists at this point: it was committed before the
        // final App commit. Capture its real inode, then allocate the
        // replacement while that file still exists so the filesystem cannot
        // hand the just-freed inode out again.
        let owned_inode = fs::metadata(&recipe_destination).unwrap().ino();
        fs::write(&replacement_source, fs::read(&recipe_destination).unwrap()).unwrap();
        let replacement_inode = fs::metadata(&replacement_source)
            .unwrap()
            .ino();
        assert_ne!(
            owned_inode, replacement_inode,
            "the replacement is a distinct inode"
        );
        // Swap the same bytes in under the new inode; nothing else changes.
        fs::rename(&replacement_source, &recipe_destination).unwrap();
        assert_eq!(
            fs::metadata(&recipe_destination).unwrap().ino(),
            replacement_inode
        );
        observed = Some((owned_inode, replacement_inode));
    })
    .expect_err("the replaced owned Recipe must refuse the final App commit");

    let GeneratedPairPersistFailure::RecipeCommitted { rollback, .. } = failure else {
        panic!("expected the Recipe-first failure shape: {failure:?}");
    };
    let (owned_inode, replacement_inode) = observed.expect("the hook observed both inodes");
    assert_ne!(owned_inode, replacement_inode);
    assert_eq!(
        fs::metadata(&recipe_destination).unwrap().ino(),
        replacement_inode,
        "the same-bytes file under a fresh inode is preserved"
    );
    assert!(
        !rollback.recipe_removed,
        "a different inode is not provably this publication's file: {rollback:?}"
    );
    assert!(rollback.recipe_recovery_required, "{rollback:?}");
    assert_eq!(
        fs::read_to_string(&recipe_destination).unwrap(),
        fixture.recipe_canonical_yaml,
        "identical bytes are preserved for manual recovery"
    );
}

#[test]
fn generated_pair_publication_refuses_a_recipe_deleted_after_its_commit() {
    let catalog = catalog();
    let root = catalog.path().canonicalize().unwrap();
    let fixture = pair_fixture(root.as_path(), "com.example.player");
    let opened = write_and_open_app(&fixture);
    let snapshot = admit(&fixture, root.as_path(), Some(opened));
    let app_destination = fixture.app_destination.clone();
    let recipe_destination = fixture.recipe_destination.clone();
    let app_before = fs::read_to_string(&app_destination).unwrap();

    let failure = persist_generated_app_pair_with(&snapshot, || {
        // An external process removes the Recipe this publication committed
        // before the App is committed.
        fs::remove_file(&recipe_destination).unwrap();
    })
    .expect_err("the missing owned Recipe must refuse the final App commit");

    match failure {
        GeneratedPairPersistFailure::RecipeCommitted { error, rollback } => {
            assert_eq!(error.code, "app_definition_stale_state", "{error:?}");
            assert!(
                !rollback.recipe_removed,
                "the publication did not remove the already-absent Recipe: {rollback:?}"
            );
            assert!(
                !rollback.recipe_recovery_required,
                "an already-absent owned Recipe leaves nothing to recover: {rollback:?}"
            );
        }
        other => panic!("expected the Recipe-first failure shape: {other:?}"),
    }
    assert!(!recipe_destination.exists());
    assert_eq!(
        fs::read_to_string(&app_destination).unwrap(),
        app_before,
        "the App is never replaced when the committed Recipe disappeared"
    );
}

#[test]
fn generated_pair_publication_accepts_its_own_recipe_addition_with_an_unchanged_dependent_recipe() {
    let catalog = catalog();
    let root = catalog.path().canonicalize().unwrap();
    let fixture = pair_fixture(root.as_path(), "com.example.player");
    let opened = write_and_open_app(&fixture);
    let affected_recipe = write_affected_recipe(root.as_path(), &fixture);
    let snapshot = admit(&fixture, root.as_path(), Some(opened));

    let outcome = persist_generated_app_pair(&snapshot).expect("publication succeeds");
    assert!(outcome.app_replaced);
    assert!(fixture.recipe_destination.exists());
    assert_eq!(
        fs::read_to_string(&fixture.recipe_destination).unwrap(),
        fixture.recipe_canonical_yaml
    );
    assert_eq!(
        fs::read_to_string(&fixture.app_destination).unwrap(),
        authored_models::emit_app_definition_yaml(&fixture.app).unwrap()
    );
    assert!(
        affected_recipe.exists(),
        "the captured dependent Recipe is untouched"
    );
}
