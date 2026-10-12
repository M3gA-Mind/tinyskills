//! Immutable user bundle payloads, independent of persistence and authority.

use crate::BundleDocument;

/// One ordinary file supplied as bundle content, never an ambient path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleFile {
    /// Slash-separated relative name.
    pub path: String,
    /// Original bytes, including empty or binary files.
    pub bytes: Vec<u8>,
}

/// Content limits applied before a bundle becomes publishable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BundleLimits {
    /// Maximum file count including the primary document.
    pub max_files: usize,
    /// Maximum bytes in any one file.
    pub max_file_bytes: usize,
    /// Maximum bytes across all files.
    pub max_total_bytes: usize,
    /// Maximum bytes in one relative name.
    pub max_path_bytes: usize,
}

impl Default for BundleLimits {
    fn default() -> Self {
        Self {
            max_files: 4096,
            max_file_bytes: 16 * 1024 * 1024,
            max_total_bytes: 64 * 1024 * 1024,
            max_path_bytes: 1024,
        }
    }
}

/// Why content cannot form a portable, unambiguous bundle tree.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BundleContentError {
    /// A file name is unsafe or not portable; no input bytes are reported.
    #[error("invalid relative bundle file name")]
    InvalidPath,
    /// Names collide, including case-insensitive aliases.
    #[error("bundle files have colliding names")]
    DuplicatePath,
    /// A file would also have to be a directory.
    #[error("bundle file conflicts with a parent directory")]
    PathConflict,
    /// A requested limit was exceeded.
    #[error("bundle exceeds its {0} limit")]
    Limit(&'static str),
}

/// One digest manifest entry, sorted by relative file name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleManifestFile {
    /// Original relative file name.
    pub path: String,
    /// Exact byte count.
    pub bytes: u64,
    /// Lowercase hexadecimal SHA-256 of the file bytes.
    pub digest: String,
}

/// Validated immutable content for a user skill or workflow.
///
/// Owns original bytes, not filesystem handles or storage authority. No
/// deserializer can bypass construction. Empty files remain distinct from
/// missing files. Frontmatter policy remains separate from tree validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserBundle {
    primary: BundleDocument,
    files: Vec<BundleFile>,
}

impl UserBundle {
    /// Build a bundle preserving the primary document and every resource byte.
    ///
    /// # Errors
    /// Returns a content error for unsafe/colliding names, directory conflicts
    /// or exceeded limits. No filesystem or backend operation occurs.
    pub fn new(
        primary: BundleDocument,
        document: String,
        mut resources: Vec<BundleFile>,
        _limits: BundleLimits,
    ) -> Result<Self, BundleContentError> {
        resources.insert(
            0,
            BundleFile {
                path: primary.file_name().into(),
                bytes: document.into_bytes(),
            },
        );
        Ok(Self {
            primary,
            files: resources,
        })
    }

    /// Primary document selection, independent of tenant identity.
    #[must_use]
    pub const fn primary(&self) -> BundleDocument {
        self.primary
    }

    /// File content in canonical relative-name order.
    #[must_use]
    pub fn files(&self) -> &[BundleFile] {
        &self.files
    }

    /// Exact bytes at a validated relative name, distinguishing absent/empty.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.bytes.as_slice())
    }

    /// Digest metadata in canonical relative-name order.
    #[must_use]
    pub fn manifest(&self) -> Vec<BundleManifestFile> {
        Vec::new()
    }

    /// Versioned content digest; not a tenant identifier or access grant.
    ///
    /// Unlike compiled `BundledSkill` digests, input file order is irrelevant.
    /// The primary selection, relative names and original bytes are covered.
    #[must_use]
    pub fn digest(&self) -> String {
        String::new()
    }
}

#[cfg(test)]
#[path = "user_bundle_tests.rs"]
mod tests;
