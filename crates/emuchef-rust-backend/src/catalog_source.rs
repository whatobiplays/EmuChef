//! Catalog-source resolution for product planning and discovery.
//!
//! A source resolves to an immutable local snapshot. Planner and executor code
//! consume only that snapshot, so a future cached remote source can populate a
//! local directory without changing planning semantics. This module performs
//! no networking or update checks.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CATALOG_DIRECTORIES: &[&str] = &["apps", "recipes", "device_profiles", "device_plans"];

/// Stable source classifications exposed by the additive product protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogSourceKind {
    Bundled,
    LocalDirectory,
    /// Reserved for a future resolver that materializes a verified remote
    /// catalog into a local cache. Phase 0 deliberately has no implementation.
    CachedRemote,
}

/// Optional content integrity, independent from source identity and version.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogContentDigest {
    pub algorithm: String,
    pub value: String,
}

impl CatalogContentDigest {
    pub fn sha256(value: impl Into<String>) -> Self {
        Self {
            algorithm: "sha256".to_string(),
            value: value.into(),
        }
    }
}

/// Identity metadata carried by product-facing catalog and plan DTOs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogIdentity {
    pub source_kind: CatalogSourceKind,
    pub source_id: String,
    pub version: Option<String>,
    pub cache_key: Option<String>,
    pub content_digest: Option<CatalogContentDigest>,
}

/// A source-neutral, locally readable catalog snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogSnapshot {
    root: PathBuf,
    identity: Option<CatalogIdentity>,
}

impl CatalogSnapshot {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn identity(&self) -> Option<&CatalogIdentity> {
        self.identity.as_ref()
    }

    /// Adapt the legacy `authoredRoot` contract without inventing product
    /// identity or integrity metadata.
    pub fn legacy_local(root: impl AsRef<Path>) -> Result<Self, CatalogSourceError> {
        resolve_root(root.as_ref()).map(|root| Self {
            root,
            identity: None,
        })
    }
}

/// Resolver boundary implemented by bundled and local-directory sources.
pub trait CatalogSource {
    fn resolve(&self) -> Result<CatalogSnapshot, CatalogSourceError>;
}

/// Phase 0 source backed by an already materialized local directory.
#[derive(Clone, Debug)]
pub struct LocalCatalogSource {
    root: PathBuf,
    identity: CatalogIdentity,
}

impl LocalCatalogSource {
    pub fn new(root: impl Into<PathBuf>, identity: CatalogIdentity) -> Self {
        Self {
            root: root.into(),
            identity,
        }
    }
}

impl CatalogSource for LocalCatalogSource {
    fn resolve(&self) -> Result<CatalogSnapshot, CatalogSourceError> {
        if self.identity.source_kind == CatalogSourceKind::CachedRemote {
            return Err(CatalogSourceError::new(
                "catalog_source_unsupported",
                "Catalog source kind 'cached_remote' is reserved and is not implemented.",
            ));
        }
        if self.identity.source_id.trim().is_empty() {
            return Err(CatalogSourceError::new(
                "catalog_source_invalid",
                "Catalog source id must be a non-empty string.",
            ));
        }

        let root = resolve_root(&self.root)?;
        if let Some(expected) = &self.identity.content_digest {
            validate_expected_digest(expected)?;
            let actual = compute_catalog_sha256(&root)?;
            if !actual.eq_ignore_ascii_case(&expected.value) {
                return Err(CatalogSourceError::new(
                    "catalog_integrity_mismatch",
                    "Resolved catalog content does not match the supplied SHA-256 digest.",
                ));
            }
        }

        Ok(CatalogSnapshot {
            root,
            identity: Some(self.identity.clone()),
        })
    }
}

/// Stable catalog resolution failure used by protocol and planning adapters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogSourceError {
    code: &'static str,
    message: String,
}

impl CatalogSourceError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for CatalogSourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CatalogSourceError {}

fn resolve_root(root: &Path) -> Result<PathBuf, CatalogSourceError> {
    root.canonicalize().map_err(|_| {
        CatalogSourceError::new(
            "catalog_unavailable",
            "Catalog snapshot root is unavailable or unreadable.",
        )
    })
}

fn validate_expected_digest(digest: &CatalogContentDigest) -> Result<(), CatalogSourceError> {
    if digest.algorithm != "sha256"
        || digest.value.len() != 64
        || !digest.value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(CatalogSourceError::new(
            "catalog_integrity_invalid",
            "Catalog content digest must be a 64-character SHA-256 hexadecimal value.",
        ));
    }
    Ok(())
}

/// Hash authored YAML files and app configuration templates in normalized path order.
///
/// Each record is framed as `<path-length>:<path><content-length>:<content>`.
/// Length framing prevents path/content boundary ambiguity while preserving raw
/// file bytes as the integrity input.
pub fn compute_catalog_sha256(root: &Path) -> Result<String, CatalogSourceError> {
    let mut files = Vec::new();
    for directory in CATALOG_DIRECTORIES {
        let path = root.join(directory);
        let metadata = fs::symlink_metadata(&path).map_err(|_| {
            CatalogSourceError::new(
                "catalog_unavailable",
                "Catalog snapshot is missing a required authored directory.",
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(CatalogSourceError::new(
                "catalog_unavailable",
                "Catalog snapshot is missing a required authored directory.",
            ));
        }
        collect_catalog_files(root, &path, &mut files)?;
    }
    let mut files = files
        .into_iter()
        .map(|path| {
            let relative = path.strip_prefix(root).map_err(|_| {
                CatalogSourceError::new(
                    "catalog_integrity_failed",
                    "Catalog file could not be normalized beneath the snapshot root.",
                )
            })?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            Ok((relative, path))
        })
        .collect::<Result<Vec<_>, CatalogSourceError>>()?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    for (relative, path) in files {
        let content = fs::read(&path).map_err(|_| {
            CatalogSourceError::new(
                "catalog_integrity_failed",
                "Catalog file could not be read for integrity verification.",
            )
        })?;
        hasher.update(relative.len().to_string().as_bytes());
        hasher.update(b":");
        hasher.update(relative.as_bytes());
        hasher.update(content.len().to_string().as_bytes());
        hasher.update(b":");
        hasher.update(&content);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    )
}

fn collect_catalog_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), CatalogSourceError> {
    let entries = fs::read_dir(directory).map_err(|_| {
        CatalogSourceError::new(
            "catalog_unavailable",
            "Catalog snapshot directory is unreadable.",
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|_| {
            CatalogSourceError::new(
                "catalog_integrity_failed",
                "Catalog snapshot entry is unreadable.",
            )
        })?;
        let file_type = entry.file_type().map_err(|_| {
            CatalogSourceError::new(
                "catalog_integrity_failed",
                "Catalog snapshot entry type is unavailable.",
            )
        })?;
        let path = entry.path();
        if file_type.is_symlink() {
            return Err(CatalogSourceError::new(
                "catalog_integrity_failed",
                "Catalog snapshot must not contain symlinks.",
            ));
        }
        if file_type.is_dir() {
            collect_catalog_files(root, &path, files)?;
        } else if file_type.is_file() && entry.file_name() == ".gitkeep" {
            continue;
        } else if file_type.is_file()
            && (is_yaml(&path) || is_app_configuration_template_asset(root, &path))
        {
            files.push(path);
        } else {
            return Err(CatalogSourceError::new(
                "catalog_integrity_failed",
                "Catalog snapshot contains an unsupported entry.",
            ));
        }
    }
    Ok(())
}

fn is_app_configuration_template_asset(root: &Path, path: &Path) -> bool {
    let Some(relative) = path.strip_prefix(root).ok().and_then(Path::to_str) else {
        return false;
    };
    let components = relative.split('/').collect::<Vec<_>>();
    components.len() >= 3
        && components[0] == "apps"
        && components[1].bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
        && components[1]
            .bytes()
            .any(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && Path::new(relative)
            .extension()
            .and_then(|value| value.to_str())
            == Some("cfg")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for directory in CATALOG_DIRECTORIES {
            fs::create_dir_all(root.path().join(directory)).unwrap();
        }
        fs::write(root.path().join("recipes/example.yaml"), "example\n").unwrap();
        root
    }

    #[test]
    fn local_sources_resolve_identity_and_verify_optional_content_digest() {
        let root = root();
        let digest = compute_catalog_sha256(root.path()).unwrap();
        let identity = CatalogIdentity {
            source_kind: CatalogSourceKind::Bundled,
            source_id: "bundled.test".to_string(),
            version: Some("1".to_string()),
            cache_key: Some("test-cache".to_string()),
            content_digest: Some(CatalogContentDigest::sha256(digest)),
        };
        let snapshot = LocalCatalogSource::new(root.path(), identity.clone())
            .resolve()
            .unwrap();
        assert_eq!(snapshot.identity(), Some(&identity));
        assert_eq!(snapshot.root(), root.path().canonicalize().unwrap());
    }

    #[test]
    fn catalog_digest_tracks_app_configuration_template_assets() {
        let root = root();
        let template_dir = root.path().join("apps/retroarch");
        fs::create_dir_all(&template_dir).unwrap();
        let template = template_dir.join("default.cfg");
        fs::write(&template, "video_threaded=false\n").unwrap();

        let first = compute_catalog_sha256(root.path()).unwrap();
        fs::write(&template, "video_threaded=true\n").unwrap();
        let second = compute_catalog_sha256(root.path()).unwrap();

        assert_ne!(
            first, second,
            "template bytes must be bound by the catalog digest"
        );
    }

    #[test]
    fn reserved_remote_and_bad_integrity_are_rejected() {
        let root = root();
        let mut identity = CatalogIdentity {
            source_kind: CatalogSourceKind::CachedRemote,
            source_id: "remote".to_string(),
            version: None,
            cache_key: None,
            content_digest: None,
        };
        assert_eq!(
            LocalCatalogSource::new(root.path(), identity.clone())
                .resolve()
                .unwrap_err()
                .code(),
            "catalog_source_unsupported"
        );
        identity.source_kind = CatalogSourceKind::LocalDirectory;
        identity.content_digest = Some(CatalogContentDigest::sha256("0".repeat(64)));
        assert_eq!(
            LocalCatalogSource::new(root.path(), identity)
                .resolve()
                .unwrap_err()
                .code(),
            "catalog_integrity_mismatch"
        );
    }
}
