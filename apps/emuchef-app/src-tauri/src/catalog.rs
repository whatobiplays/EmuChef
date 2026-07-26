//! Source-neutral bundled catalog resolution and resource verification.

#[cfg(any(test, all(debug_assertions, feature = "real-execution")))]
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

const REQUIRED_DIRECTORIES: &[&str] = &["apps", "device_plans", "device_profiles", "recipes"];
#[cfg(any(test, all(debug_assertions, feature = "real-execution")))]
const QUALIFICATION_FRAGMENT_FILES: &[&str] = &[
    "apps/phase-6c-qualification.yaml",
    "device_plans/phase-6c-qualification.yaml",
    "device_profiles/phase-6c-qualification.yaml",
    "recipes/phase-6c-qualification.yaml",
];

#[derive(Clone, Debug)]
pub struct CatalogDescriptor {
    root: PathBuf,
    identity: CatalogIdentityDto,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogIdentityDto {
    source_kind: &'static str,
    source_id: &'static str,
    version: Option<String>,
    content_digest: CatalogDigestDto,
}

#[derive(Clone, Debug, Serialize)]
pub struct CatalogDigestDto {
    algorithm: &'static str,
    value: String,
}

impl CatalogDescriptor {
    pub fn resolve(app: &AppHandle) -> Result<Self, String> {
        let ordinary_root = if cfg!(debug_assertions) {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .find(|candidate| candidate.join("authored").is_dir())
                .map(|candidate| candidate.join("authored"))
                .ok_or_else(|| "Development catalog root was not found.".to_string())?
        } else {
            app.path()
                .resource_dir()
                .map_err(|_| "Packaged resource directory is unavailable.".to_string())?
                .join("catalog")
        };
        let resolved = (ordinary_root, "emuchef.phase1.bundled", "phase1-bundled-1");
        #[cfg(all(debug_assertions, feature = "real-execution"))]
        let resolved = if qualification_catalog_requested(
            true,
            true,
            std::env::var("EMUCHEF_RUN_REAL_ADB_TESTS").ok().as_deref(),
            std::env::var("EMUCHEF_PHASE_6C_QUALIFICATION_CATALOG")
                .ok()
                .as_deref(),
        ) {
            let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .find(|candidate| candidate.join("tests/fixtures/phase-6c/non-root").is_dir())
                .ok_or_else(|| "Phase 6C qualification fixture root was not found.".to_string())?;
            let fragment_root = repository_root.join("tests/fixtures/phase-6c/non-root/recipe");
            let cache_root = app
                .path()
                .app_cache_dir()
                .map_err(|_| "Application cache directory is unavailable.".to_string())?;
            (
                build_qualification_catalog_overlay(&resolved.0, &fragment_root, &cache_root)?,
                "emuchef.phase6c.qualification",
                "phase6c-qualification-1",
            )
        } else {
            resolved
        };
        let (root, source_id, version) = resolved;
        let digest = verify_and_digest(&root)?;
        Ok(Self {
            root,
            identity: CatalogIdentityDto {
                source_kind: "bundled",
                source_id,
                version: Some(version.to_string()),
                content_digest: CatalogDigestDto {
                    algorithm: "sha256",
                    value: digest,
                },
            },
        })
    }

    pub fn internal_payload(&self) -> Value {
        json!({
            "root": self.root,
            "sourceKind": self.identity.source_kind,
            "sourceId": self.identity.source_id,
            "version": self.identity.version,
            "contentDigest": self.identity.content_digest,
        })
    }

    pub fn public_identity(&self) -> CatalogIdentityDto {
        self.identity.clone()
    }

    pub fn digest(&self) -> &str {
        &self.identity.content_digest.value
    }
}

/// Verifies a materialized bundled catalog and returns its canonical digest.
pub(crate) fn verify_and_digest(root: &Path) -> Result<String, String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "Bundled catalog root is unavailable.".to_string())?;
    let mut files = Vec::new();
    for directory in REQUIRED_DIRECTORIES {
        let path = canonical_root.join(directory);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| format!("Bundled catalog directory '{directory}' is missing."))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Bundled catalog directory '{directory}' must not be a symlink."
            ));
        }
        if !metadata.file_type().is_dir() {
            return Err(format!(
                "Bundled catalog directory '{directory}' is missing."
            ));
        }
        collect_files(&canonical_root, &path, &mut files)?;
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err("Bundled catalog contains no files.".to_string());
    }
    let mut hasher = Sha256::new();
    for (relative, path) in files {
        let bytes =
            fs::read(path).map_err(|_| "Bundled catalog file is unreadable.".to_string())?;
        hasher.update(relative.len().to_string().as_bytes());
        hasher.update(b":");
        hasher.update(relative.as_bytes());
        hasher.update(bytes.len().to_string().as_bytes());
        hasher.update(b":");
        hasher.update(bytes);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|_| "Bundled catalog directory is unreadable.".to_string())?
    {
        let entry = entry.map_err(|_| "Bundled catalog entry is unreadable.".to_string())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "Bundled catalog entry type is unavailable.".to_string())?;
        if file_type.is_symlink() {
            return Err("Bundled catalog must not contain symlinks.".to_string());
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file()
            && path.file_name().and_then(|value| value.to_str()) == Some(".gitkeep")
        {
            continue;
        } else if file_type.is_file()
            && matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("yaml" | "yml")
            )
        {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "Bundled catalog entry escaped its root.".to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            files.push((relative, path));
        } else {
            return Err("Bundled catalog contains an unsupported entry type.".to_string());
        }
    }
    Ok(())
}

#[cfg(any(test, all(debug_assertions, feature = "real-execution")))]
fn qualification_catalog_requested(
    debug_build: bool,
    real_execution: bool,
    global_opt_in: Option<&str>,
    catalog_opt_in: Option<&str>,
) -> bool {
    debug_build && real_execution && global_opt_in == Some("1") && catalog_opt_in == Some("1")
}

#[cfg(any(test, all(debug_assertions, feature = "real-execution")))]
fn build_qualification_catalog_overlay(
    ordinary_root: &Path,
    fragment_root: &Path,
    app_cache_root: &Path,
) -> Result<PathBuf, String> {
    reject_symlink_root(ordinary_root, "Development catalog root")?;
    reject_symlink_root(fragment_root, "Qualification catalog fragment")?;
    for relative in QUALIFICATION_FRAGMENT_FILES {
        let path = fragment_root.join(relative);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "Qualification catalog fragment is incomplete.".to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Qualification catalog fragment contains an unsafe file.".to_string());
        }
    }

    fs::create_dir_all(app_cache_root)
        .map_err(|_| "Application cache directory could not be prepared.".to_string())?;
    reject_symlink_root(app_cache_root, "Application cache root")?;
    let canonical_cache = app_cache_root
        .canonicalize()
        .map_err(|_| "Application cache root is unavailable.".to_string())?;
    let overlay = canonical_cache.join("phase-6c-qualification-catalog");
    let staging = canonical_cache.join("phase-6c-qualification-catalog.building");
    for candidate in [&overlay, &staging] {
        if let Ok(metadata) = fs::symlink_metadata(candidate) {
            if metadata.file_type().is_symlink() {
                return Err("Qualification catalog cache path must not be a symlink.".to_string());
            }
            if !metadata.is_dir() {
                return Err("Qualification catalog cache path is invalid.".to_string());
            }
            fs::remove_dir_all(candidate)
                .map_err(|_| "Stale qualification catalog could not be removed.".to_string())?;
        }
    }

    let build_result = (|| {
        let ordinary_ids = catalog_ids(ordinary_root)?;
        let fragment_ids = catalog_ids(fragment_root)?;
        if !ordinary_ids.is_disjoint(&fragment_ids) {
            return Err(
                "Qualification catalog identity collides with the ordinary catalog.".to_string(),
            );
        }
        copy_catalog_tree(ordinary_root, &staging, false)?;
        copy_catalog_tree(fragment_root, &staging, true)?;
        verify_and_digest(&staging)?;
        fs::rename(&staging, &overlay)
            .map_err(|_| "Qualification catalog could not be activated.".to_string())?;
        Ok(overlay.clone())
    })();
    if build_result.is_err() {
        let _ = fs::remove_dir_all(&staging);
        let _ = fs::remove_dir_all(&overlay);
    }
    build_result
}

#[cfg(any(test, all(debug_assertions, feature = "real-execution")))]
fn reject_symlink_root(root: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(root).map_err(|_| format!("{label} is unavailable."))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} must be a real directory."));
    }
    Ok(())
}

#[cfg(any(test, all(debug_assertions, feature = "real-execution")))]
fn copy_catalog_tree(
    source: &Path,
    destination: &Path,
    reject_collisions: bool,
) -> Result<(), String> {
    reject_symlink_root(source, "Catalog source")?;
    fs::create_dir_all(destination)
        .map_err(|_| "Qualification catalog destination could not be created.".to_string())?;
    for entry in fs::read_dir(source).map_err(|_| "Catalog source is unreadable.".to_string())? {
        let entry = entry.map_err(|_| "Catalog source entry is unreadable.".to_string())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "Catalog source entry type is unavailable.".to_string())?;
        if file_type.is_symlink() {
            return Err("Catalog source must not contain symlinks.".to_string());
        }
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_catalog_tree(&entry.path(), &target, reject_collisions)?;
        } else if file_type.is_file()
            && (entry.file_name() == ".gitkeep"
                || matches!(
                    entry.path().extension().and_then(|value| value.to_str()),
                    Some("yaml" | "yml")
                ))
        {
            if reject_collisions && target.exists() {
                return Err(
                    "Qualification catalog path collides with the ordinary catalog.".to_string(),
                );
            }
            fs::copy(entry.path(), target)
                .map_err(|_| "Qualification catalog file could not be copied.".to_string())?;
        } else {
            return Err("Catalog source contains an unsupported file.".to_string());
        }
    }
    Ok(())
}

#[cfg(any(test, all(debug_assertions, feature = "real-execution")))]
fn catalog_ids(root: &Path) -> Result<HashSet<String>, String> {
    let mut files = Vec::new();
    for directory in REQUIRED_DIRECTORIES {
        collect_files(root, &root.join(directory), &mut files)?;
    }
    let mut ids = HashSet::new();
    for (_, path) in files {
        let bytes = fs::read(&path).map_err(|_| "Catalog file is unreadable.".to_string())?;
        let value = serde_yaml::from_slice::<serde_yaml::Value>(&bytes)
            .map_err(|_| "Catalog YAML is invalid.".to_string())?;
        let id = value
            .get("id")
            .and_then(serde_yaml::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "Catalog entry is missing an identity.".to_string())?;
        if !ids.insert(id.to_string()) {
            return Err("Catalog contains a duplicate identity.".to_string());
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_catalog(root: &Path, prefix: &str) {
        for directory in REQUIRED_DIRECTORIES {
            fs::create_dir_all(root.join(directory)).unwrap();
            fs::write(
                root.join(directory)
                    .join(format!("{prefix}-{directory}.yaml")),
                format!("schema_version: 1\nkind: fixture\nid: {prefix}.{directory}\n"),
            )
            .unwrap();
        }
    }

    fn write_qualification_fragment(root: &Path) {
        for relative in QUALIFICATION_FRAGMENT_FILES {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                path,
                format!(
                    "schema_version: 1\nkind: fixture\nid: {}\n",
                    relative.replace(['/', '.'], "-")
                ),
            )
            .unwrap();
        }
    }

    #[test]
    fn digest_requires_all_resource_directories_and_is_stable() {
        let temp = tempfile::tempdir().unwrap();
        for directory in REQUIRED_DIRECTORIES {
            fs::create_dir(temp.path().join(directory)).unwrap();
            fs::write(temp.path().join(directory).join("one.yaml"), directory).unwrap();
            fs::write(temp.path().join(directory).join(".gitkeep"), "\n").unwrap();
        }
        let first = verify_and_digest(temp.path()).unwrap();
        let second = verify_and_digest(temp.path()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn qualification_catalog_requires_every_exact_gate() {
        assert!(qualification_catalog_requested(
            true,
            true,
            Some("1"),
            Some("1")
        ));
        for values in [
            (false, true, Some("1"), Some("1")),
            (true, false, Some("1"), Some("1")),
            (true, true, None, Some("1")),
            (true, true, Some("true"), Some("1")),
            (true, true, Some("1"), None),
            (true, true, Some("1"), Some("true")),
        ] {
            assert!(!qualification_catalog_requested(
                values.0, values.1, values.2, values.3
            ));
        }
    }

    #[test]
    fn qualification_overlay_rebuilds_normal_catalog_with_fixed_fragment() {
        let temp = tempfile::tempdir().unwrap();
        let ordinary = temp.path().join("ordinary");
        let fragment = temp.path().join("fragment");
        let cache = temp.path().join("cache");
        write_catalog(&ordinary, "ordinary");
        write_qualification_fragment(&fragment);
        fs::create_dir_all(cache.join("phase-6c-qualification-catalog/recipes")).unwrap();
        fs::write(
            cache.join("phase-6c-qualification-catalog/recipes/stale.yaml"),
            "stale",
        )
        .unwrap();

        let overlay = build_qualification_catalog_overlay(&ordinary, &fragment, &cache).unwrap();

        assert_eq!(
            overlay,
            cache
                .canonicalize()
                .unwrap()
                .join("phase-6c-qualification-catalog")
        );
        assert!(!overlay.join("recipes/stale.yaml").exists());
        for relative in QUALIFICATION_FRAGMENT_FILES {
            assert!(overlay.join(relative).is_file());
        }
        assert_ne!(
            verify_and_digest(&ordinary).unwrap(),
            verify_and_digest(&overlay).unwrap()
        );
    }

    #[test]
    fn committed_qualification_fragment_loads_through_the_product_catalog() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .find(|candidate| candidate.join("tests/fixtures/phase-6c/non-root").is_dir())
            .unwrap();
        let ordinary = repository_root.join("authored");
        let fragment = repository_root.join("tests/fixtures/phase-6c/non-root/recipe");
        let cache = tempfile::tempdir().unwrap();
        let overlay =
            build_qualification_catalog_overlay(&ordinary, &fragment, cache.path()).unwrap();
        let digest = verify_and_digest(&overlay).unwrap();
        let catalog = json!({
            "root": overlay,
            "sourceKind": "bundled",
            "sourceId": "emuchef.phase6c.qualification",
            "version": "phase6c-qualification-1",
            "contentDigest": {
                "algorithm": "sha256",
                "value": digest,
            },
        });

        let response = emuchef_rust_backend::request::handle_one_shot_value(json!({
            "type": "describeCatalog",
            "payload": { "catalog": catalog.clone() },
        }));

        assert_eq!(response["ok"], true, "{response:#}");
        assert!(response["result"]["recipes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|recipe| recipe["id"] == "phase6c.qualification.non_root"));

        let planned = emuchef_rust_backend::request::handle_one_shot_value(json!({
            "type": "planConfiguration",
            "payload": {
                "catalog": catalog,
                "devicePlan": "phase6c.qualification.plan.non_root",
                "selectedRecipes": ["phase6c.qualification.non_root"],
                "bindings": {
                    "phase6c.qualification.non_root/fixture_apk":
                        repository_root.join("tests/fixtures/phase-6c/non-root/android-fixture/fixture.apk"),
                    "phase6c.qualification.non_root/single_file":
                        repository_root.join("tests/fixtures/phase-6c/non-root/corpus/source/single-file.txt"),
                    "phase6c.qualification.non_root/nested_directory":
                        repository_root.join("tests/fixtures/phase-6c/non-root/corpus/source/nested"),
                    "phase6c.qualification.non_root/fixture_archive":
                        repository_root.join("tests/fixtures/phase-6c/non-root/corpus/archive.zip"),
                },
                "targetDevice": {
                    "serial": "TEST-SERIAL-NOT-PROJECTED",
                    "manufacturer": "Qualification",
                    "model": "Non-root fixture",
                    "androidApiLevel": 35,
                },
            },
        }));
        assert_eq!(planned["ok"], true, "{planned:#}");
        assert!(!planned["result"]["plan"].is_null(), "{planned:#}");
        assert_eq!(
            planned["result"]["plan"]["source"]["catalog"]["sourceId"],
            "emuchef.phase6c.qualification"
        );
        let types = planned["result"]["plan"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["type"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            [
                "install_apk",
                "copy_files",
                "copy_files",
                "extract_archive",
                "grant_permissions",
                "launch_app",
                "force_stop_app",
            ]
        );
    }

    #[test]
    fn qualification_overlay_rejects_missing_fragment_and_identity_collisions() {
        let temp = tempfile::tempdir().unwrap();
        let ordinary = temp.path().join("ordinary");
        let fragment = temp.path().join("fragment");
        let cache = temp.path().join("cache");
        write_catalog(&ordinary, "ordinary");
        write_qualification_fragment(&fragment);
        fs::remove_file(fragment.join(QUALIFICATION_FRAGMENT_FILES[0])).unwrap();
        assert!(build_qualification_catalog_overlay(&ordinary, &fragment, &cache).is_err());

        write_qualification_fragment(&fragment);
        fs::write(
            fragment.join("recipes/phase-6c-qualification.yaml"),
            "schema_version: 1\nkind: fixture\nid: ordinary.recipes\n",
        )
        .unwrap();
        assert!(build_qualification_catalog_overlay(&ordinary, &fragment, &cache).is_err());
    }

    #[test]
    fn qualification_overlay_rejects_unsupported_fragment_files_without_cache_state() {
        let temp = tempfile::tempdir().unwrap();
        let ordinary = temp.path().join("ordinary");
        let fragment = temp.path().join("fragment");
        let cache = temp.path().join("cache");
        write_catalog(&ordinary, "ordinary");
        write_qualification_fragment(&fragment);
        fs::write(fragment.join("recipes/unexpected.txt"), "unsupported").unwrap();

        assert!(build_qualification_catalog_overlay(&ordinary, &fragment, &cache).is_err());
        assert!(!cache.join("phase-6c-qualification-catalog").exists());
        assert!(!cache
            .join("phase-6c-qualification-catalog.building")
            .exists());
    }

    #[cfg(unix)]
    #[test]
    fn qualification_overlay_rejects_symlinked_sources() {
        let temp = tempfile::tempdir().unwrap();
        let ordinary = temp.path().join("ordinary");
        let fragment = temp.path().join("fragment");
        let cache = temp.path().join("cache");
        write_catalog(&ordinary, "ordinary");
        write_qualification_fragment(&fragment);
        let recipe = fragment.join("recipes/phase-6c-qualification.yaml");
        fs::remove_file(&recipe).unwrap();
        std::os::unix::fs::symlink(ordinary.join("recipes/ordinary-recipes.yaml"), &recipe)
            .unwrap();

        assert!(build_qualification_catalog_overlay(&ordinary, &fragment, &cache).is_err());
        assert!(!cache.join("phase-6c-qualification-catalog").exists());
    }

    #[cfg(unix)]
    #[test]
    fn qualification_overlay_rejects_symlinked_cache_destination() {
        let temp = tempfile::tempdir().unwrap();
        let ordinary = temp.path().join("ordinary");
        let fragment = temp.path().join("fragment");
        let cache = temp.path().join("cache");
        let outside = temp.path().join("outside");
        write_catalog(&ordinary, "ordinary");
        write_qualification_fragment(&fragment);
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, cache.join("phase-6c-qualification-catalog")).unwrap();

        assert!(build_qualification_catalog_overlay(&ordinary, &fragment, &cache).is_err());
        assert!(outside.read_dir().unwrap().next().is_none());
    }

    #[test]
    fn digest_rejects_missing_resource_directory() {
        let temp = tempfile::tempdir().unwrap();
        assert!(verify_and_digest(temp.path()).unwrap_err().contains("apps"));
    }

    #[test]
    fn digest_rejects_symlinks_and_unsupported_resource_entries() {
        let temp = tempfile::tempdir().unwrap();
        for directory in REQUIRED_DIRECTORIES {
            fs::create_dir(temp.path().join(directory)).unwrap();
            fs::write(temp.path().join(directory).join("one.yaml"), directory).unwrap();
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                temp.path().join("apps/one.yaml"),
                temp.path().join("recipes/linked.yaml"),
            )
            .unwrap();
            assert!(verify_and_digest(temp.path())
                .unwrap_err()
                .contains("symlinks"));
            fs::remove_file(temp.path().join("recipes/linked.yaml")).unwrap();
        }
        fs::write(temp.path().join("recipes/unexpected.txt"), "unsupported").unwrap();
        assert!(verify_and_digest(temp.path())
            .unwrap_err()
            .contains("unsupported"));
    }

    #[cfg(unix)]
    #[test]
    fn digest_rejects_required_directory_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        for directory in REQUIRED_DIRECTORIES {
            if *directory == "apps" {
                std::os::unix::fs::symlink(outside.path(), temp.path().join(directory)).unwrap();
            } else {
                fs::create_dir(temp.path().join(directory)).unwrap();
                fs::write(temp.path().join(directory).join("one.yaml"), directory).unwrap();
            }
        }
        assert!(verify_and_digest(temp.path())
            .unwrap_err()
            .contains("must not be a symlink"));
    }
}
