//! Host-bound skills stores and the owner-open migration.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tinyskills::{StoredCatalog, is_safe_segment};
use tinystoragedrivers_api::{DocumentStore, Storage};
use tinystoragedrivers_types::{ErrorKind, Precondition, Query, StorageError, WriteOptions};

use crate::blob_bundles::BlobBundles;
use crate::bundle_store::DocumentBundleStore;
use crate::document_store::{CatalogBody, DocumentCatalogStore};
use crate::run_log::StreamRunLogStore;
use crate::schema::{
    CATALOG_COLLECTION, CATALOG_SCHEMA, CATALOG_V1_COLLECTION, MIGRATIONS_COLLECTION, collections,
};

const CATALOG_MIGRATION_ID: &str = "catalogs_v2";
const MIGRATION_PAGE_SIZE: usize = 128;

/// Errors returned while opening the skills owner's storage.
#[derive(Debug, thiserror::Error)]
pub enum SkillsStorageError {
    /// A host storage port rejected declaration or migration I/O.
    #[error("skills storage operation failed: {0}")]
    Storage(#[from] StorageError),
    /// A catalog could not be serialized during migration.
    #[error("skills catalog could not be serialized: {0}")]
    Serialization(#[from] serde_json::Error),
    /// A legacy catalog could not be represented without losing data.
    #[error("a legacy skills catalog conflicts with its expected migrated representation")]
    MigrationConflict,
}

/// All skills stores bound to the host's explicitly supplied backend scope.
#[derive(Clone, Debug)]
pub struct SkillsStorage {
    catalogs: DocumentCatalogStore,
    bundles: DocumentBundleStore,
    bundle_tree: BlobBundles,
    run_logs: StreamRunLogStore,
}

impl SkillsStorage {
    /// Ensure the skills collections and migrate legacy catalogs on the
    /// supplied scope-bound storage handles.
    ///
    /// This call never creates a backend or chooses a scope. Each handle in
    /// `storage` remains bound to the scope selected by its host.
    ///
    /// # Errors
    /// Returns an error if collection setup or migration I/O fails, or when a
    /// corrupt current catalog would prevent safe preservation of its source.
    pub async fn open(storage: Storage) -> Result<Self, SkillsStorageError> {
        for spec in collections() {
            storage.ensure_collection(&spec).await?;
        }
        migrate_catalogs(storage.documents().as_ref()).await?;
        Ok(Self {
            catalogs: DocumentCatalogStore::new(Arc::clone(storage.documents())),
            bundles: DocumentBundleStore::new(
                Arc::clone(storage.documents()),
                Arc::clone(storage.blobs()),
            ),
            bundle_tree: BlobBundles::new(Arc::clone(storage.blobs())),
            run_logs: StreamRunLogStore::new(Arc::clone(storage.streams())),
        })
    }

    /// Access the fetched registry catalog store.
    #[must_use]
    pub fn catalogs(&self) -> &DocumentCatalogStore {
        &self.catalogs
    }

    /// Access the CAS-backed user-bundle store.
    #[must_use]
    pub fn bundles(&self) -> &DocumentBundleStore {
        &self.bundles
    }

    /// Access the host-materialized blob-tree bundle store.
    #[must_use]
    pub fn bundle_tree(&self) -> &BlobBundles {
        &self.bundle_tree
    }

    /// Access the append-only run-log store bound to this host scope.
    #[must_use]
    pub fn run_logs(&self) -> &StreamRunLogStore {
        &self.run_logs
    }
}

async fn migrate_catalogs(documents: &dyn DocumentStore) -> Result<(), SkillsStorageError> {
    if let Some(row) = documents
        .get(MIGRATIONS_COLLECTION, CATALOG_MIGRATION_ID)
        .await?
    {
        let marker = serde_json::from_value::<MigrationMarker>(row.doc)
            .map_err(|_| SkillsStorageError::MigrationConflict)?;
        // An incomplete receipt is advisory only; the full scan below retries
        // any rows left behind by the interrupted open.
        let MigrationMarker { version, .. } = marker;
        if version != 1 {
            return Err(SkillsStorageError::MigrationConflict);
        }
    }
    let mut query = Query::all().limit(MIGRATION_PAGE_SIZE);
    loop {
        let page = documents.query(CATALOG_V1_COLLECTION, &query).await?;
        for source in page.items {
            migrate_catalog(documents, source).await?;
        }
        let Some(cursor) = page.next else {
            break;
        };
        query = Query::all().limit(MIGRATION_PAGE_SIZE).after(cursor);
    }

    // This receipt records that this owner completed a scan, but deliberately
    // never suppresses a later scan: a legacy writer may add a row after it.
    // Per-row destination CAS and source-version deletion make the repeated
    // scan idempotent without hiding late rows.
    let marker = MigrationMarker {
        version: 1,
        _complete: true,
    };
    documents
        .put_with(
            MIGRATIONS_COLLECTION,
            CATALOG_MIGRATION_ID,
            serde_json::to_value(marker)?,
            Precondition::None,
            WriteOptions::new(),
        )
        .await?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct MigrationMarker {
    version: u32,
    #[serde(rename = "complete")]
    _complete: bool,
}

async fn migrate_catalog(
    documents: &dyn DocumentStore,
    source: tinystoragedrivers_types::Versioned<serde_json::Value>,
) -> Result<(), SkillsStorageError> {
    if !is_safe_segment(&source.id) {
        return Err(SkillsStorageError::MigrationConflict);
    }
    let stored = serde_json::from_value::<StoredCatalog>(source.doc.clone())
        .map_err(|_| SkillsStorageError::MigrationConflict)?;
    if stored.format != StoredCatalog::FORMAT || serde_json::to_value(&stored)? != source.doc {
        return Err(SkillsStorageError::MigrationConflict);
    }
    let expected_body = serde_json::to_value(CatalogBody::from_stored(&stored))?;
    let options = source.meta.expires_ms.map_or_else(
        || WriteOptions::new().schema(CATALOG_SCHEMA),
        |expires| {
            WriteOptions::new()
                .schema(CATALOG_SCHEMA)
                .expires_ms(expires)
        },
    );
    match documents
        .put_with(
            CATALOG_COLLECTION,
            &source.id,
            expected_body.clone(),
            Precondition::Absent,
            options,
        )
        .await
    {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::Conflict => {}
        Err(error) => return Err(error.into()),
    }
    let current = documents.get(CATALOG_COLLECTION, &source.id).await?;
    let exact_destination = current.is_some_and(|row| {
        row.meta.schema == CATALOG_SCHEMA
            && row.meta.expires_ms == source.meta.expires_ms
            && row.doc == expected_body
    });
    if !exact_destination {
        return Err(SkillsStorageError::MigrationConflict);
    }

    match documents
        .delete(
            CATALOG_V1_COLLECTION,
            &source.id,
            Precondition::Version(source.version),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
