//! Transport primitives used by artifact resolution.
//!
//! The executor deliberately does not perform source I/O itself. Transport
//! implementations copy or download bytes while the resolver owns destination
//! selection and sandbox policy.

use std::fs;
use std::path::Path;

use crate::artifact_resolver::ArtifactResolveError;

/// Transfer an artifact source to a resolver-selected destination.
pub(crate) trait ArtifactTransport {
    fn download(&self, source: &Path, destination: &Path) -> Result<(), ArtifactResolveError>;
}

/// Local-file transport preserving the executor's existing copy semantics.
#[derive(Debug, Default)]
pub(crate) struct LocalFileTransport;

impl ArtifactTransport for LocalFileTransport {
    fn download(&self, source: &Path, destination: &Path) -> Result<(), ArtifactResolveError> {
        fs::copy(source, destination)
            .map(|_| ())
            .map_err(|error| ArtifactResolveError::new(error.to_string()))
    }
}
