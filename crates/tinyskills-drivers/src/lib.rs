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

mod blob_bundles;
mod document_store;

pub use blob_bundles::{BlobBundleError, BlobBundles};
pub use document_store::{CATALOG_COLLECTION, DocumentCatalogStore};
