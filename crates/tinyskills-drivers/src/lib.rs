//! `tinyskills` on the host's `tinystoragedrivers` ports.
//!
//! The crate links the storage ports only, never a database client: the host
//! picks the backend (`SQLite`, `MongoDB`, memory) and the tenant scope when it
//! builds the handles, and passes them in.
//!
//! - [`DocumentCatalogStore`]: fetched registry catalogs as documents, a
//!   [`CatalogStore`](tinyskills::CatalogStore) for
//!   [`SkillRegistry`](tinyskills::SkillRegistry).
//! - [`BlobBundles`]: skill bundle files as a blob tree, materialized to a
//!   cache directory for discovery with [`BlobBundles::materialize`].
//! - [`DocumentBundleStore`]: CAS-protected user-bundle heads referencing
//!   immutable generations under [`BUNDLE_GENERATION_PREFIX`], separate from
//!   the deletable [`BUNDLE_PREFIX`] tree.
//! - [`collections`]: pure document collection declarations for the `skills`
//!   owner.
//! - [`SkillsStorage`]: explicit host-bound stores; opening it ensures the
//!   declared collections and migrates legacy registry catalogs.

mod blob_bundles;
mod bundle_store;
mod document_store;
mod run_log;
mod schema;
mod storage;

pub use blob_bundles::{BUNDLE_GENERATION_PREFIX, BUNDLE_PREFIX, BlobBundleError, BlobBundles};
pub use bundle_store::{BUNDLE_COLLECTION, DocumentBundleStore};
pub use document_store::DocumentCatalogStore;
pub use run_log::{RunLogStoreError, StreamRunLogStore};
pub use schema::{CATALOG_COLLECTION, MIGRATIONS_COLLECTION, blob_prefixes, collections};
pub use storage::{SkillsStorage, SkillsStorageError};

#[cfg(test)]
#[path = "schema_tests.rs"]
mod schema_tests;
