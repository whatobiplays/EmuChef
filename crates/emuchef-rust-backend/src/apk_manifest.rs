//! Production-compiled internal boundary for hostile APK manifest inspection.
//!
//! `rusty-axml` has known panic paths and permissive behavior for some malformed
//! binary XML, so every parser and traversal call must remain behind the
//! structural validation, integer normalization, size limits, redacted error
//! mapping, and panic boundary implemented here. Parser-owned types must not
//! escape this module.
//!
//! Phase 5B2 defines a stable crate-internal model but deliberately has no
//! production caller. Later phases may integrate the inspector without
//! weakening this mandatory defensive wrapper.

use rusty_axml::parser::{Axml, XmlElement};
use std::fmt;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use zip::ZipArchive;

const ANDROID_MANIFEST_ENTRY: &str = "AndroidManifest.xml";
const ANDROID_XML_CHUNK_TYPE: u16 = 0x0003;
const ANDROID_XML_HEADER_SIZE: u16 = 8;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const ZIP_EOCD_MIN_BYTES: usize = 22;
const ZIP_MAX_COMMENT_BYTES: usize = u16::MAX as usize;

/// Stable manifest facts owned by the backend inspection boundary.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ApkManifestFacts {
    pub(crate) package_name: String,
    pub(crate) version_code: Option<String>,
    pub(crate) version_name: Option<String>,
    pub(crate) min_sdk_version: Option<String>,
    pub(crate) target_sdk_version: Option<String>,
    pub(crate) permissions: Vec<ApkPermissionDeclaration>,
}

/// A requested permission declared directly below the manifest root.
#[derive(Clone, Debug, Ord, PartialOrd, PartialEq, Eq)]
pub(crate) struct ApkPermissionDeclaration {
    pub(crate) name: String,
    pub(crate) kind: ApkPermissionDeclarationKind,
    pub(crate) max_sdk_version: Option<String>,
}

/// The two Android manifest elements that request permissions.
#[derive(Clone, Copy, Debug, Ord, PartialOrd, PartialEq, Eq)]
pub(crate) enum ApkPermissionDeclarationKind {
    UsesPermission,
    UsesPermissionSdk23,
}

/// Stable, redacted failures exposed by the backend inspection boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApkManifestError {
    ZipInvalid,
    ManifestMissing,
    ManifestTooLarge,
    ManifestInvalid,
    PackageMissing,
    InspectionFailed,
}

impl ApkManifestError {
    /// Return the stable application error code for this failure.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::ZipInvalid => "apk_zip_invalid",
            Self::ManifestMissing => "apk_manifest_missing",
            Self::ManifestTooLarge => "apk_manifest_too_large",
            Self::ManifestInvalid => "apk_manifest_invalid",
            Self::PackageMissing => "apk_package_missing",
            Self::InspectionFailed => "apk_manifest_inspection_failed",
        }
    }
}

impl fmt::Display for ApkManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZipInvalid => "APK archive is invalid.",
            Self::ManifestMissing => "APK archive does not contain AndroidManifest.xml.",
            Self::ManifestTooLarge => "APK manifest exceeds the 4 MiB size limit.",
            Self::ManifestInvalid => "APK manifest is invalid.",
            Self::PackageMissing => "APK manifest package name is missing.",
            Self::InspectionFailed => "APK manifest could not be inspected.",
        })
    }
}

impl std::error::Error for ApkManifestError {}

/// Inspect the single root manifest entry in an APK without reading the whole
/// archive into memory.
pub(crate) fn inspect_apk_manifest(path: &Path) -> Result<ApkManifestFacts, ApkManifestError> {
    let mut file = File::open(path).map_err(|_| ApkManifestError::InspectionFailed)?;
    let declared_entry_count = declared_zip_entry_count(&mut file)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| ApkManifestError::ZipInvalid)?;
    let mut archive = ZipArchive::new(file).map_err(|_| ApkManifestError::ZipInvalid)?;
    if archive.len() != declared_entry_count {
        return Err(ApkManifestError::ZipInvalid);
    }
    let manifest_index = unique_manifest_index(&mut archive)?;
    let manifest = archive
        .by_index(manifest_index)
        .map_err(|_| ApkManifestError::ZipInvalid)?;
    let declared_manifest_size = manifest.size();
    let manifest_bytes = read_manifest_bounded(manifest, Some(declared_manifest_size))?;
    validate_axml_container(&manifest_bytes)?;
    parse_manifest_contained(manifest_bytes)
}

fn declared_zip_entry_count(file: &mut File) -> Result<usize, ApkManifestError> {
    let file_size = file
        .metadata()
        .map_err(|_| ApkManifestError::ZipInvalid)?
        .len();
    let tail_size = file_size.min((ZIP_EOCD_MIN_BYTES + ZIP_MAX_COMMENT_BYTES) as u64);
    file.seek(SeekFrom::End(-i64::try_from(tail_size).unwrap_or(i64::MAX)))
        .map_err(|_| ApkManifestError::ZipInvalid)?;
    let mut tail =
        Vec::with_capacity(usize::try_from(tail_size).map_err(|_| ApkManifestError::ZipInvalid)?);
    file.read_to_end(&mut tail)
        .map_err(|_| ApkManifestError::ZipInvalid)?;

    let signature = [0x50, 0x4b, 0x05, 0x06];
    for start in (0..=tail.len().saturating_sub(ZIP_EOCD_MIN_BYTES)).rev() {
        let record = &tail[start..];
        if record.len() < ZIP_EOCD_MIN_BYTES || !record.starts_with(&signature) {
            continue;
        }
        let comment_length = u16::from_le_bytes([record[20], record[21]]) as usize;
        if ZIP_EOCD_MIN_BYTES + comment_length != record.len() {
            continue;
        }
        let entries_on_disk = u16::from_le_bytes([record[8], record[9]]);
        let total_entries = u16::from_le_bytes([record[10], record[11]]);
        if entries_on_disk != total_entries || total_entries == u16::MAX {
            return Err(ApkManifestError::ZipInvalid);
        }
        return Ok(total_entries as usize);
    }
    Err(ApkManifestError::ZipInvalid)
}

fn unique_manifest_index(archive: &mut ZipArchive<File>) -> Result<usize, ApkManifestError> {
    let mut manifest_index = None;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| ApkManifestError::ZipInvalid)?;
        if entry.name() == ANDROID_MANIFEST_ENTRY && manifest_index.replace(index).is_some() {
            return Err(ApkManifestError::ZipInvalid);
        }
    }
    manifest_index.ok_or(ApkManifestError::ManifestMissing)
}

fn read_manifest_bounded<R: Read>(
    reader: R,
    declared_size: Option<u64>,
) -> Result<Vec<u8>, ApkManifestError> {
    if declared_size.is_some_and(|size| size > MAX_MANIFEST_BYTES as u64) {
        return Err(ApkManifestError::ManifestTooLarge);
    }

    let capacity = declared_size
        .and_then(|size| usize::try_from(size).ok())
        .unwrap_or_default();
    let mut bytes = Vec::with_capacity(capacity);
    reader
        .take(MAX_MANIFEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ApkManifestError::ManifestInvalid)?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ApkManifestError::ManifestTooLarge);
    }
    Ok(bytes)
}

fn validate_axml_container(bytes: &[u8]) -> Result<(), ApkManifestError> {
    let Some(header) = bytes.get(..ANDROID_XML_HEADER_SIZE as usize) else {
        return Err(ApkManifestError::ManifestInvalid);
    };
    let chunk_type = u16::from_le_bytes([header[0], header[1]]);
    let header_size = u16::from_le_bytes([header[2], header[3]]);
    let total_size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if chunk_type != ANDROID_XML_CHUNK_TYPE
        || header_size != ANDROID_XML_HEADER_SIZE
        || usize::try_from(total_size).ok() != Some(bytes.len())
    {
        return Err(ApkManifestError::ManifestInvalid);
    }
    Ok(())
}

fn parse_manifest_contained(manifest_bytes: Vec<u8>) -> Result<ApkManifestFacts, ApkManifestError> {
    match catch_unwind(AssertUnwindSafe(|| {
        let parsed = rusty_axml::parse_from_cursor(Cursor::new(manifest_bytes))
            .map_err(|_| ApkManifestError::ManifestInvalid)?;
        extract_manifest_facts(&parsed)
    })) {
        Ok(result) => result,
        Err(_) => Err(ApkManifestError::ManifestInvalid),
    }
}

fn extract_manifest_facts(parsed: &Axml) -> Result<ApkManifestFacts, ApkManifestError> {
    let root = parsed.root().borrow();
    if root.element_type() != "manifest" {
        return Err(ApkManifestError::ManifestInvalid);
    }
    let package_name = root
        .get_attr("package")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ApkManifestError::PackageMissing)?
        .to_string();
    let version_code = integer_attribute(&root, "android:versionCode", false)?;
    let version_name = root.get_attr("android:versionName").map(str::to_string);

    let mut min_sdk_version = None;
    let mut target_sdk_version = None;
    let mut uses_sdk_seen = false;
    let mut permissions = Vec::new();
    for child in root.children() {
        let child = child.borrow();
        match child.element_type() {
            "uses-sdk" => {
                if uses_sdk_seen {
                    return Err(ApkManifestError::ManifestInvalid);
                }
                uses_sdk_seen = true;
                min_sdk_version = integer_attribute(&child, "android:minSdkVersion", true)?;
                target_sdk_version = integer_attribute(&child, "android:targetSdkVersion", true)?;
            }
            "uses-permission" => permissions.push(permission_declaration(
                &child,
                ApkPermissionDeclarationKind::UsesPermission,
            )?),
            "uses-permission-sdk-23" => permissions.push(permission_declaration(
                &child,
                ApkPermissionDeclarationKind::UsesPermissionSdk23,
            )?),
            _ => {}
        }
    }

    permissions.sort();
    permissions.dedup();
    Ok(ApkManifestFacts {
        package_name,
        version_code,
        version_name,
        min_sdk_version,
        target_sdk_version,
        permissions,
    })
}

fn permission_declaration(
    element: &XmlElement,
    kind: ApkPermissionDeclarationKind,
) -> Result<ApkPermissionDeclaration, ApkManifestError> {
    let name = element
        .get_attr("android:name")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ApkManifestError::ManifestInvalid)?
        .to_string();
    Ok(ApkPermissionDeclaration {
        name,
        kind,
        max_sdk_version: integer_attribute(element, "android:maxSdkVersion", false)?,
    })
}

fn integer_attribute(
    element: &XmlElement,
    name: &str,
    allow_sdk_codename: bool,
) -> Result<Option<String>, ApkManifestError> {
    element
        .get_attr(name)
        .map(|value| normalize_integer_value(value, allow_sdk_codename))
        .transpose()
}

fn normalize_integer_value(
    value: &str,
    allow_sdk_codename: bool,
) -> Result<String, ApkManifestError> {
    const TYPED_DECIMAL_PREFIX: &str = "(type 0x10) 0x";
    const TYPED_HEXADECIMAL_PREFIX: &str = "(type 0x11) 0x";
    if let Some(encoded) = value
        .strip_prefix(TYPED_DECIMAL_PREFIX)
        .or_else(|| value.strip_prefix(TYPED_HEXADECIMAL_PREFIX))
    {
        return u32::from_str_radix(encoded, 16)
            .map(|number| number.to_string())
            .map_err(|_| ApkManifestError::ManifestInvalid);
    }

    if value
        .parse::<u32>()
        .ok()
        .map(|number| number.to_string())
        .as_deref()
        == Some(value)
    {
        return Ok(value.to_string());
    }
    if allow_sdk_codename
        && value
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Ok(value.to_string());
    }
    Err(ApkManifestError::ManifestInvalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    const ANDROID_NAMESPACE: &str = "http://schemas.android.com/apk/res/android";
    const NO_INDEX: u32 = u32::MAX;

    #[derive(Clone, Copy)]
    enum FixtureValue<'a> {
        Raw(&'a str),
        Typed { data_type: u8, data: u32 },
    }

    #[derive(Clone, Copy)]
    struct FixtureAttribute<'a> {
        android_namespace: bool,
        name: &'a str,
        value: FixtureValue<'a>,
    }

    struct FixtureElement<'a> {
        tag: &'a str,
        attributes: Vec<FixtureAttribute<'a>>,
        children: Vec<FixtureElement<'a>>,
    }

    fn raw(name: &'static str, value: &'static str) -> FixtureAttribute<'static> {
        FixtureAttribute {
            android_namespace: name != "package",
            name,
            value: FixtureValue::Raw(value),
        }
    }

    fn typed(name: &'static str, data_type: u8, data: u32) -> FixtureAttribute<'static> {
        FixtureAttribute {
            android_namespace: true,
            name,
            value: FixtureValue::Typed { data_type, data },
        }
    }

    fn element(
        tag: &'static str,
        attributes: Vec<FixtureAttribute<'static>>,
        children: Vec<FixtureElement<'static>>,
    ) -> FixtureElement<'static> {
        FixtureElement {
            tag,
            attributes,
            children,
        }
    }

    fn valid_manifest() -> FixtureElement<'static> {
        element(
            "manifest",
            vec![
                raw("package", "com.example.qualified"),
                typed("versionCode", 0x10, 42),
                raw("versionName", "2.5"),
            ],
            vec![
                element(
                    "uses-sdk",
                    vec![
                        typed("minSdkVersion", 0x11, 23),
                        typed("targetSdkVersion", 0x10, 35),
                    ],
                    vec![],
                ),
                element(
                    "uses-permission-sdk-23",
                    vec![
                        raw("name", "android.permission.CAMERA"),
                        typed("maxSdkVersion", 0x11, 34),
                    ],
                    vec![],
                ),
                element(
                    "uses-permission",
                    vec![raw("name", "android.permission.ACCESS_FINE_LOCATION")],
                    vec![],
                ),
                element(
                    "uses-permission",
                    vec![raw("name", "android.permission.CAMERA")],
                    vec![],
                ),
                element(
                    "uses-permission",
                    vec![raw("name", "android.permission.CAMERA")],
                    vec![],
                ),
                element(
                    "application",
                    vec![],
                    vec![element(
                        "uses-permission",
                        vec![raw("name", "android.permission.NESTED_SHOULD_BE_IGNORED")],
                        vec![],
                    )],
                ),
            ],
        )
    }

    #[test]
    fn apk_manifest_extracts_and_normalizes_qualified_facts() {
        let workspace = TempDir::new().expect("temporary workspace should be created");
        let apk = write_apk(
            &workspace,
            &[(ANDROID_MANIFEST_ENTRY, build_axml(&valid_manifest()))],
        );

        let facts = inspect_apk_manifest(&apk).expect("qualified manifest should parse");

        assert_eq!(facts.package_name, "com.example.qualified");
        assert_eq!(facts.version_code.as_deref(), Some("42"));
        assert_eq!(facts.version_name.as_deref(), Some("2.5"));
        assert_eq!(facts.min_sdk_version.as_deref(), Some("23"));
        assert_eq!(facts.target_sdk_version.as_deref(), Some("35"));
        assert_eq!(
            facts.permissions,
            [
                ApkPermissionDeclaration {
                    name: "android.permission.ACCESS_FINE_LOCATION".to_string(),
                    kind: ApkPermissionDeclarationKind::UsesPermission,
                    max_sdk_version: None,
                },
                ApkPermissionDeclaration {
                    name: "android.permission.CAMERA".to_string(),
                    kind: ApkPermissionDeclarationKind::UsesPermission,
                    max_sdk_version: None,
                },
                ApkPermissionDeclaration {
                    name: "android.permission.CAMERA".to_string(),
                    kind: ApkPermissionDeclarationKind::UsesPermissionSdk23,
                    max_sdk_version: Some("34".to_string()),
                },
            ]
        );
    }

    #[test]
    fn apk_manifest_accepts_missing_optional_metadata_and_sdk_codename() {
        let workspace = TempDir::new().expect("temporary workspace should be created");
        let manifest = element(
            "manifest",
            vec![raw("package", "com.example.preview")],
            vec![
                element(
                    "uses-sdk",
                    vec![raw("targetSdkVersion", "VanillaIceCream")],
                    vec![],
                ),
                element(
                    "application",
                    vec![],
                    vec![element(
                        "uses-sdk",
                        vec![raw("targetSdkVersion", "999")],
                        vec![],
                    )],
                ),
            ],
        );
        let apk = write_apk(
            &workspace,
            &[(ANDROID_MANIFEST_ENTRY, build_axml(&manifest))],
        );

        let facts = inspect_apk_manifest(&apk).expect("optional facts may be absent");

        assert_eq!(facts.version_code, None);
        assert_eq!(facts.version_name, None);
        assert_eq!(facts.min_sdk_version, None);
        assert_eq!(facts.target_sdk_version.as_deref(), Some("VanillaIceCream"));
        assert!(facts.permissions.is_empty());
    }

    #[test]
    fn apk_manifest_rejects_duplicate_direct_child_uses_sdk_declarations() {
        let workspace = TempDir::new().expect("temporary workspace should be created");
        let cases = [
            vec![
                element("uses-sdk", vec![], vec![]),
                element("uses-sdk", vec![], vec![]),
            ],
            vec![
                element("uses-sdk", vec![], vec![]),
                element("uses-sdk", vec![raw("minSdkVersion", "23")], vec![]),
            ],
            vec![
                element("uses-sdk", vec![raw("targetSdkVersion", "35")], vec![]),
                element("uses-sdk", vec![], vec![]),
            ],
            vec![
                element("uses-sdk", vec![raw("minSdkVersion", "23")], vec![]),
                element("uses-sdk", vec![raw("targetSdkVersion", "35")], vec![]),
            ],
        ];

        for children in cases {
            let manifest = element(
                "manifest",
                vec![raw("package", "com.example.duplicate-sdk")],
                children,
            );
            let apk = write_apk(
                &workspace,
                &[(ANDROID_MANIFEST_ENTRY, build_axml(&manifest))],
            );
            assert_error(&apk, ApkManifestError::ManifestInvalid);
        }
    }

    #[test]
    fn apk_manifest_rejects_invalid_zip_missing_and_duplicate_entries() {
        let workspace = TempDir::new().expect("temporary workspace should be created");
        let invalid = workspace.path().join("private-invalid.apk");
        fs::write(&invalid, b"not a zip").expect("invalid fixture should be written");
        assert_error(&invalid, ApkManifestError::ZipInvalid);

        let missing = write_apk(&workspace, &[("payload.bin", vec![1, 2, 3])]);
        assert_error(&missing, ApkManifestError::ManifestMissing);

        let duplicate = write_duplicate_manifest_apk(&workspace);
        assert_error(&duplicate, ApkManifestError::ZipInvalid);
    }

    #[test]
    fn apk_manifest_rejects_malformed_and_panicking_axml() {
        let workspace = TempDir::new().expect("temporary workspace should be created");
        let malformed = axml_with_payload(&[0x01, 0x00, 0x08, 0x00, 0x08, 0x00, 0x00, 0x00]);
        let malformed_apk = write_apk(&workspace, &[(ANDROID_MANIFEST_ENTRY, malformed)]);
        assert_error(&malformed_apk, ApkManifestError::ManifestInvalid);

        let panicking = axml_with_payload(&[0x01, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00]);
        let panicking_apk = write_apk(&workspace, &[(ANDROID_MANIFEST_ENTRY, panicking)]);
        assert_error(&panicking_apk, ApkManifestError::ManifestInvalid);

        for invalid_header in [Vec::new(), vec![0; 8], axml_with_declared_size(&[], 9)] {
            let apk = write_apk(&workspace, &[(ANDROID_MANIFEST_ENTRY, invalid_header)]);
            assert_error(&apk, ApkManifestError::ManifestInvalid);
        }
    }

    #[test]
    fn apk_manifest_enforces_declared_and_streamed_size_limits() {
        let workspace = TempDir::new().expect("temporary workspace should be created");
        let oversized = vec![0; MAX_MANIFEST_BYTES + 1];
        let apk = write_apk(&workspace, &[(ANDROID_MANIFEST_ENTRY, oversized.clone())]);
        assert_error(&apk, ApkManifestError::ManifestTooLarge);

        let reader = UnknownSizeReader(Cursor::new(oversized));
        assert_eq!(
            read_manifest_bounded(reader, None),
            Err(ApkManifestError::ManifestTooLarge)
        );
    }

    #[test]
    fn apk_manifest_rejects_missing_package_and_invalid_permission() {
        let workspace = TempDir::new().expect("temporary workspace should be created");
        for package in [None, Some("   ")] {
            let attributes = package
                .map(|value| vec![raw("package", value)])
                .unwrap_or_default();
            let manifest = element("manifest", attributes, vec![]);
            let apk = write_apk(
                &workspace,
                &[(ANDROID_MANIFEST_ENTRY, build_axml(&manifest))],
            );
            assert_error(&apk, ApkManifestError::PackageMissing);
        }

        let manifest = element(
            "manifest",
            vec![raw("package", "com.example.invalid")],
            vec![element("uses-permission", vec![], vec![])],
        );
        let apk = write_apk(
            &workspace,
            &[(ANDROID_MANIFEST_ENTRY, build_axml(&manifest))],
        );
        assert_error(&apk, ApkManifestError::ManifestInvalid);
    }

    #[test]
    fn apk_manifest_rejects_unknown_or_malformed_integer_renderings() {
        let workspace = TempDir::new().expect("temporary workspace should be created");
        let manifest = element(
            "manifest",
            vec![
                raw("package", "com.example.invalid"),
                typed("versionCode", 0x04, 1),
            ],
            vec![],
        );
        let apk = write_apk(
            &workspace,
            &[(ANDROID_MANIFEST_ENTRY, build_axml(&manifest))],
        );
        assert_error(&apk, ApkManifestError::ManifestInvalid);

        assert_eq!(
            normalize_integer_value("(type 0x10) nope", false),
            Err(ApkManifestError::ManifestInvalid)
        );
        assert_eq!(
            normalize_integer_value("001", false),
            Err(ApkManifestError::ManifestInvalid)
        );
        assert_eq!(
            normalize_integer_value("Vanilla-Ice-Cream", true),
            Err(ApkManifestError::ManifestInvalid)
        );
    }

    #[test]
    fn apk_manifest_errors_are_stable_and_redacted() {
        let matrix = [
            (
                ApkManifestError::ZipInvalid,
                "apk_zip_invalid",
                "APK archive is invalid.",
            ),
            (
                ApkManifestError::ManifestMissing,
                "apk_manifest_missing",
                "APK archive does not contain AndroidManifest.xml.",
            ),
            (
                ApkManifestError::ManifestTooLarge,
                "apk_manifest_too_large",
                "APK manifest exceeds the 4 MiB size limit.",
            ),
            (
                ApkManifestError::ManifestInvalid,
                "apk_manifest_invalid",
                "APK manifest is invalid.",
            ),
            (
                ApkManifestError::PackageMissing,
                "apk_package_missing",
                "APK manifest package name is missing.",
            ),
            (
                ApkManifestError::InspectionFailed,
                "apk_manifest_inspection_failed",
                "APK manifest could not be inspected.",
            ),
        ];
        for (error, code, message) in matrix {
            assert_eq!(error.code(), code);
            assert_eq!(error.to_string(), message);
            for secret in [
                "/Users/private/source.apk",
                "https://publisher.invalid/app.apk",
                "secret-source-name.apk",
                "unexpected XML chunk type",
            ] {
                assert!(!format!("{error:?} {error}").contains(secret));
            }
        }

        let missing_path = Path::new("/Users/private/secret-source-name.apk");
        assert_error(missing_path, ApkManifestError::InspectionFailed);
    }

    fn assert_error(path: &Path, expected: ApkManifestError) {
        let error = inspect_apk_manifest(path).expect_err("fixture should be rejected");
        assert_eq!(error, expected);
    }

    struct UnknownSizeReader(Cursor<Vec<u8>>);

    impl Read for UnknownSizeReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buffer)
        }
    }

    fn write_apk(workspace: &TempDir, entries: &[(&str, Vec<u8>)]) -> std::path::PathBuf {
        let path = workspace
            .path()
            .join(format!("fixture-{}.apk", entries.len()));
        let file = File::create(&path).expect("APK fixture should be writable");
        let mut archive = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        for (name, contents) in entries {
            archive
                .start_file(*name, options)
                .expect("APK entry should start");
            archive
                .write_all(contents)
                .expect("APK entry should be writable");
        }
        archive.finish().expect("APK fixture should finish");
        path
    }

    fn write_duplicate_manifest_apk(workspace: &TempDir) -> std::path::PathBuf {
        const FIRST_PLACEHOLDER: &str = "AndroidManifest.one";
        const SECOND_PLACEHOLDER: &str = "AndroidManifest.two";
        let path = write_apk(
            workspace,
            &[
                (FIRST_PLACEHOLDER, build_axml(&valid_manifest())),
                (SECOND_PLACEHOLDER, build_axml(&valid_manifest())),
            ],
        );
        let mut bytes = fs::read(&path).expect("duplicate-entry fixture should be readable");
        for placeholder in [FIRST_PLACEHOLDER, SECOND_PLACEHOLDER] {
            replace_all_equal_length(
                &mut bytes,
                placeholder.as_bytes(),
                ANDROID_MANIFEST_ENTRY.as_bytes(),
            );
        }
        fs::write(&path, bytes).expect("duplicate-entry fixture should be writable");
        path
    }

    fn replace_all_equal_length(bytes: &mut [u8], from: &[u8], to: &[u8]) {
        assert_eq!(from.len(), to.len());
        let mut start = 0;
        while start + from.len() <= bytes.len() {
            if &bytes[start..start + from.len()] == from {
                bytes[start..start + to.len()].copy_from_slice(to);
                start += from.len();
            } else {
                start += 1;
            }
        }
    }

    fn build_axml(root: &FixtureElement<'_>) -> Vec<u8> {
        let mut strings = vec!["android".to_string(), ANDROID_NAMESPACE.to_string()];
        collect_strings(root, &mut strings);

        let mut payload = string_pool_chunk(&strings);
        payload.extend(namespace_chunk(0x0100, &strings));
        append_element(root, &strings, &mut payload);
        payload.extend(namespace_chunk(0x0101, &strings));
        axml_with_payload(&payload)
    }

    fn collect_strings(element: &FixtureElement<'_>, strings: &mut Vec<String>) {
        push_string(strings, element.tag);
        for attribute in &element.attributes {
            push_string(strings, attribute.name);
            if let FixtureValue::Raw(value) = attribute.value {
                push_string(strings, value);
            }
        }
        for child in &element.children {
            collect_strings(child, strings);
        }
    }

    fn push_string(strings: &mut Vec<String>, value: &str) {
        if !strings.iter().any(|existing| existing == value) {
            strings.push(value.to_string());
        }
    }

    fn string_index(strings: &[String], value: &str) -> u32 {
        strings
            .iter()
            .position(|existing| existing == value)
            .and_then(|index| u32::try_from(index).ok())
            .expect("fixture string should have an index")
    }

    fn string_pool_chunk(strings: &[String]) -> Vec<u8> {
        let mut string_data = Vec::new();
        let mut offsets = Vec::new();
        for value in strings {
            offsets.push(u32::try_from(string_data.len()).expect("fixture offset should fit"));
            let length = u8::try_from(value.len()).expect("fixture strings should be short ASCII");
            string_data.extend([length, length]);
            string_data.extend(value.as_bytes());
            string_data.push(0);
        }
        while string_data.len() % 4 != 0 {
            string_data.push(0);
        }

        let strings_start = 28 + offsets.len() * 4;
        let chunk_size = strings_start + string_data.len();
        let mut chunk = Vec::new();
        push_u16(&mut chunk, 0x0001);
        push_u16(&mut chunk, 28);
        push_u32(
            &mut chunk,
            u32::try_from(chunk_size).expect("fixture chunk should fit"),
        );
        push_u32(
            &mut chunk,
            u32::try_from(strings.len()).expect("fixture count should fit"),
        );
        push_u32(&mut chunk, 0);
        push_u32(&mut chunk, 0x0000_0100);
        push_u32(
            &mut chunk,
            u32::try_from(strings_start).expect("fixture offset should fit"),
        );
        push_u32(&mut chunk, 0);
        for offset in offsets {
            push_u32(&mut chunk, offset);
        }
        chunk.extend(string_data);
        chunk
    }

    fn namespace_chunk(chunk_type: u16, strings: &[String]) -> Vec<u8> {
        let mut chunk = Vec::new();
        push_u16(&mut chunk, chunk_type);
        push_u16(&mut chunk, 16);
        push_u32(&mut chunk, 24);
        push_u32(&mut chunk, 1);
        push_u32(&mut chunk, NO_INDEX);
        push_u32(&mut chunk, string_index(strings, "android"));
        push_u32(&mut chunk, string_index(strings, ANDROID_NAMESPACE));
        chunk
    }

    fn append_element(element: &FixtureElement<'_>, strings: &[String], output: &mut Vec<u8>) {
        let chunk_size = 36 + element.attributes.len() * 20;
        push_u16(output, 0x0102);
        push_u16(output, 16);
        push_u32(
            output,
            u32::try_from(chunk_size).expect("fixture chunk should fit"),
        );
        push_u32(output, 1);
        push_u32(output, NO_INDEX);
        push_u32(output, NO_INDEX);
        push_u32(output, string_index(strings, element.tag));
        push_u16(output, 20);
        push_u16(output, 20);
        push_u16(
            output,
            u16::try_from(element.attributes.len()).expect("fixture count should fit"),
        );
        push_u16(output, 0);
        push_u16(output, 0);
        push_u16(output, 0);
        for attribute in &element.attributes {
            push_u32(
                output,
                if attribute.android_namespace {
                    string_index(strings, ANDROID_NAMESPACE)
                } else {
                    NO_INDEX
                },
            );
            push_u32(output, string_index(strings, attribute.name));
            let (raw_value, data_type, data) = match attribute.value {
                FixtureValue::Raw(value) => {
                    let index = string_index(strings, value);
                    (index, 0x03, index)
                }
                FixtureValue::Typed { data_type, data } => (NO_INDEX, data_type, data),
            };
            push_u32(output, raw_value);
            push_u16(output, 8);
            output.push(0);
            output.push(data_type);
            push_u32(output, data);
        }

        for child in &element.children {
            append_element(child, strings, output);
        }

        push_u16(output, 0x0103);
        push_u16(output, 16);
        push_u32(output, 24);
        push_u32(output, 1);
        push_u32(output, NO_INDEX);
        push_u32(output, NO_INDEX);
        push_u32(output, string_index(strings, element.tag));
    }

    fn axml_with_payload(payload: &[u8]) -> Vec<u8> {
        axml_with_declared_size(
            payload,
            u32::try_from(ANDROID_XML_HEADER_SIZE as usize + payload.len())
                .expect("fixture size should fit"),
        )
    }

    fn axml_with_declared_size(payload: &[u8], declared_size: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_u16(&mut bytes, ANDROID_XML_CHUNK_TYPE);
        push_u16(&mut bytes, ANDROID_XML_HEADER_SIZE);
        push_u32(&mut bytes, declared_size);
        bytes.extend(payload);
        bytes
    }

    fn push_u16(output: &mut Vec<u8>, value: u16) {
        output.extend(value.to_le_bytes());
    }

    fn push_u32(output: &mut Vec<u8>, value: u32) {
        output.extend(value.to_le_bytes());
    }
}
