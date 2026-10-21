//! Pure collection declarations for the `skills` storage owner.

use tinystoragedrivers_types::CollectionSpec;

use crate::blob_bundles::{BUNDLE_GENERATION_PREFIX, BUNDLE_PREFIX};
use crate::bundle_store::BUNDLE_COLLECTION;

/// The collection that holds one document per registry catalog.
pub const CATALOG_COLLECTION: &str = "skills.catalogs";

/// The legacy collection read during the owner migration.
pub(crate) const CATALOG_V1_COLLECTION: &str = "tinyskills_catalogs";

/// Per-owner receipts for completed skills migrations; this is not a central
/// registry and never suppresses the legacy-row scan on a later open.
pub(crate) const MIGRATIONS_COLLECTION: &str = "skills.migrations";

/// The body version of [`CATALOG_COLLECTION`].
pub(crate) const CATALOG_SCHEMA: u32 = 2;

/// Declarations the host must ensure for the skills owner.
///
/// This is a pure description: it performs no storage I/O and chooses no
/// backend or scope. The host should ensure these collections on its bound
/// document handle before calling [`crate::SkillsStorage::open`].
#[must_use]
pub fn collections() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(CATALOG_COLLECTION),
        CollectionSpec::new(BUNDLE_COLLECTION),
        CollectionSpec::new(MIGRATIONS_COLLECTION),
    ]
}

/// Blob namespaces used by the skills owner.
///
/// Blob stores do not have a collection declaration operation; these prefixes
/// are listed for hosts that need to report their complete storage layout.
#[must_use]
pub fn blob_prefixes() -> [&'static str; 2] {
    [BUNDLE_PREFIX, BUNDLE_GENERATION_PREFIX]
}
