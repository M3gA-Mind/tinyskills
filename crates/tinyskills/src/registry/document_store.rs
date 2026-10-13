//! A [`CatalogStore`] on the host's `tinystoragedrivers` document port.

use std::sync::Arc;

use tinystoragedrivers_core::{
    CollectionSpec, DocumentStore, ErrorKind, Precondition, StorageError,
};
use tokio::sync::OnceCell;

use super::error::StoreError;
use super::store::{CatalogStore, StoredCatalog};
use super::transport::BoxFuture;
use crate::is_safe_segment;

/// The collection that holds one document per registry.
pub const CATALOG_COLLECTION: &str = "tinyskills_catalogs";

/// Fetched catalogs as documents, `tinyskills_catalogs/<registry>`.
///
/// The host picks the backend (`SQLite`, `MongoDB`, memory) and the tenant scope
/// when it builds the [`DocumentStore`] handle; this store never names either.
/// A document in another format reads as absent, and one that does not decode
/// reads as [`StoreError::Corrupt`]. A failing backend surfaces as
/// [`StoreError::Io`], never as an absent catalog.
#[derive(Clone)]
pub struct DocumentCatalogStore {
    documents: Arc<dyn DocumentStore>,
    ready: Arc<OnceCell<()>>,
}

impl std::fmt::Debug for DocumentCatalogStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentCatalogStore")
            .finish_non_exhaustive()
    }
}

impl DocumentCatalogStore {
    /// A store on `documents`; the collection is declared on first use.
    #[must_use]
    pub fn new(documents: Arc<dyn DocumentStore>) -> Self {
        Self {
            documents,
            ready: Arc::new(OnceCell::new()),
        }
    }

    async fn ensure(&self) -> Result<(), StoreError> {
        self.ready
            .get_or_try_init(|| async {
                self.documents
                    .ensure_collection(&CollectionSpec::new(CATALOG_COLLECTION))
                    .await
            })
            .await
            .map_err(|error| storage_error(&error))?;
        Ok(())
    }

    fn check(registry: &str) -> Result<(), StoreError> {
        if is_safe_segment(registry) {
            Ok(())
        } else {
            Err(StoreError::InvalidId(registry.to_owned()))
        }
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
            Self::check(registry)?;
            self.ensure().await?;
            let Some(found) = self
                .documents
                .get(CATALOG_COLLECTION, registry)
                .await
                .map_err(|error| storage_error(&error))?
            else {
                return Ok(None);
            };
            let format = found
                .doc
                .get("format")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if format != u64::from(StoredCatalog::FORMAT) {
                return Ok(None);
            }
            serde_json::from_value(found.doc)
                .map(Some)
                .map_err(|error| StoreError::Corrupt(error.to_string()))
        })
    }

    fn save<'a>(
        &'a self,
        registry: &'a str,
        catalog: &'a StoredCatalog,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            Self::check(registry)?;
            self.ensure().await?;
            let doc =
                serde_json::to_value(catalog).map_err(|error| StoreError::Io(error.to_string()))?;
            self.documents
                .put(CATALOG_COLLECTION, registry, doc, Precondition::None)
                .await
                .map(|_| ())
                .map_err(|error| storage_error(&error))
        })
    }
}

#[cfg(test)]
#[path = "document_store_tests.rs"]
mod tests;
