//! Source-neutral bundled catalog resolution and resource verification.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

const REQUIRED_DIRECTORIES: &[&str] = &["apps", "device_plans", "device_profiles", "recipes"];

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
        let root = if cfg!(debug_assertions) {
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
        let digest = verify_and_digest(&root)?;
        Ok(Self {
            root,
            identity: CatalogIdentityDto {
                source_kind: "bundled",
                source_id: "emuchef.phase1.bundled",
                version: Some("phase1-bundled-1".to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

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
