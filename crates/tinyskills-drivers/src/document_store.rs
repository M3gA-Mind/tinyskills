//! A [`CatalogStore`] on the host's `tinystoragedrivers` document port.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tinystoragedrivers_core::{
    DocumentStore, ErrorKind, Key, Precondition, StorageError, WriteOptions,
};

use tinyskills::{
    BoxFuture, CatalogStore, RegistryEntry, StoreError, StoredCatalog, Validators, is_safe_segment,
};

use crate::schema::{CATALOG_COLLECTION, CATALOG_SCHEMA};

/// The stored body of one catalog (`skills.catalogs`, schema 2): the
/// [`StoredCatalog`] fields with the fetch time in epoch milliseconds and no
/// `format` field, since the document's schema version says it.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct CatalogBody {
    entries: Vec<RegistryEntry>,
    fetched_ms: i64,
    validators: Validators,
    skipped: usize,
}

impl CatalogBody {
    /// The body for `catalog`.
    pub(crate) fn from_stored(catalog: &StoredCatalog) -> Self {
        Self {
            entries: catalog.entries.clone(),
            fetched_ms: i64::try_from(catalog.fetched_at)
                .unwrap_or(i64::MAX)
                .saturating_mul(1000),
            validators: catalog.validators.clone(),
            skipped: catalog.skipped,
        }
    }

    fn into_stored(self) -> StoredCatalog {
        StoredCatalog::new(
            self.entries,
            u64::try_from(self.fetched_ms).unwrap_or(0) / 1000,
            self.validators,
            self.skipped,
        )
    }
}

/// Fetched catalogs as documents, `skills.catalogs/<registry>`.
///
/// The host picks the backend (`SQLite`, `MongoDB`, memory) and the tenant scope
/// when it builds the [`DocumentStore`] handle; this store never names either.
/// The collection is declared by [`storage_schema`](crate::storage_schema)
/// through the host's `Registry`, never by this store. A document written at
/// another schema version reads as absent, and one that does not decode reads
/// as [`StoreError::Corrupt`]. A failing backend surfaces as
/// [`StoreError::Io`], never as an absent catalog.
#[derive(Clone)]
pub struct DocumentCatalogStore {
    documents: Arc<dyn DocumentStore>,
}

impl std::fmt::Debug for DocumentCatalogStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentCatalogStore")
            .finish_non_exhaustive()
    }
}

impl DocumentCatalogStore {
    /// A store on `documents`, the tenant-scoped handle of the host's backend.
    #[must_use]
    pub fn new(documents: Arc<dyn DocumentStore>) -> Self {
        Self { documents }
    }

    fn id(registry: &str) -> Result<String, StoreError> {
        if !is_safe_segment(registry) {
            return Err(StoreError::InvalidId(registry.to_owned()));
        }
        Key::of([registry])
            .map(|key| key.as_str().to_owned())
            .map_err(|_| StoreError::InvalidId(registry.to_owned()))
    }
}

fn storage_error(error: &StorageError) -> StoreError {
    match error.kind() {
        ErrorKind::Serialization => StoreError::Corrupt(error.to_string()),
        _ => StoreError::Io(error.to_string()),
    }
}

impl CatalogStore for DocumentCatalogStore {
    fn load<'a>(
        &'a self,
        registry: &'a str,
    ) -> BoxFuture<'a, Result<Option<StoredCatalog>, StoreError>> {
        Box::pin(async move {
            let id = Self::id(registry)?;
            let Some(found) = self
                .documents
                .get(CATALOG_COLLECTION, &id)
                .await
                .map_err(|error| storage_error(&error))?
            else {
                return Ok(None);
            };
            if found.meta.schema != CATALOG_SCHEMA {
                return Ok(None);
            }
            serde_json::from_value::<CatalogBody>(found.doc)
                .map(|body| Some(body.into_stored()))
                .map_err(|error| StoreError::Corrupt(error.to_string()))
        })
    }

    fn save<'a>(
        &'a self,
        registry: &'a str,
        catalog: &'a StoredCatalog,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let id = Self::id(registry)?;
            let doc = serde_json::to_value(CatalogBody::from_stored(catalog))
                .map_err(|error| StoreError::Io(error.to_string()))?;
            self.documents
                .put_with(
                    CATALOG_COLLECTION,
                    &id,
                    doc,
                    Precondition::None,
                    WriteOptions::new().schema(CATALOG_SCHEMA),
                )
                .await
                .map(|_| ())
                .map_err(|error| storage_error(&error))
        })
    }
}

#[cfg(test)]
#[path = "document_store_tests.rs"]
mod tests;
