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
//!   immutable generations in the bundle blob tree.
//! - [`storage_schema`]: the owner `skills` declaration of both (the catalog
//!   and bundle-head collections, bundle blob prefix, and catalog migration
//!   steps, for the host's `Registry::compose`.

mod blob_bundles;
mod bundle_store;
mod document_store;
mod schema;

pub use blob_bundles::{BUNDLE_PREFIX, BlobBundleError, BlobBundles};
pub use bundle_store::{BUNDLE_COLLECTION, DocumentBundleStore};
pub use document_store::DocumentCatalogStore;
pub use schema::{CATALOG_COLLECTION, storage_schema};
