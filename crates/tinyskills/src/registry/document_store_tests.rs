#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;
use tinystoragedrivers_core::{MemoryStorage, Scope, StorageBackend};

use super::*;
use crate::registry::contract::{RegistryEntry, Validators};

fn catalog() -> StoredCatalog {
    let mut entry = RegistryEntry::default();
    entry.entry.id = "a".to_owned();
    entry.entry.name = "a".to_owned();
    let validators = Validators {
        etag: Some("\"e\"".to_owned()),
        ..Validators::default()
    };
    StoredCatalog::new(vec![entry], 42, validators, 3)
}

fn docs(scope: &str) -> Arc<dyn DocumentStore> {
    let storage = MemoryStorage::new();
    Arc::clone(
        storage
            .for_scope(&Scope::new(scope).unwrap())
            .unwrap()
            .documents(),
    )
}

#[tokio::test]
async fn round_trips_and_replaces() {
    let store = DocumentCatalogStore::new(docs("alice"));
    assert_eq!(store.load("example").await.unwrap(), None);
    store.save("example", &catalog()).await.unwrap();
    assert_eq!(store.load("example").await.unwrap(), Some(catalog()));
    let newer = StoredCatalog::new(Vec::new(), 99, Validators::default(), 0);
    store.save("example", &newer).await.unwrap();
    assert_eq!(store.load("example").await.unwrap(), Some(newer));
    assert_eq!(store.load("other").await.unwrap(), None);
    assert!(format!("{store:?}").contains("DocumentCatalogStore"));
}

#[tokio::test]
async fn shares_state_across_clones_and_arc() {
    let store = DocumentCatalogStore::new(docs("alice"));
    let copy = store.clone();
    store.save("r", &catalog()).await.unwrap();
    assert_eq!(copy.load("r").await.unwrap(), Some(catalog()));
    let shared: Arc<dyn CatalogStore> = Arc::new(copy);
    assert_eq!(shared.load("r").await.unwrap(), Some(catalog()));
}

#[tokio::test]
async fn refuses_unsafe_ids() {
    let store = DocumentCatalogStore::new(docs("alice"));
    for id in ["", "..", "a/b", "a b"] {
        assert_eq!(
            store.load(id).await,
            Err(StoreError::InvalidId(id.to_owned()))
        );
        assert_eq!(
            store.save(id, &catalog()).await,
            Err(StoreError::InvalidId(id.to_owned()))
        );
    }
}

#[tokio::test]
async fn other_formats_read_as_absent_and_bad_documents_as_corrupt() {
    let documents = docs("alice");
    let store = DocumentCatalogStore::new(Arc::clone(&documents));
    let put = |doc: serde_json::Value| {
        let documents = Arc::clone(&documents);
        async move {
            documents
                .put(CATALOG_COLLECTION, "r", doc, Precondition::None)
                .await
                .unwrap();
        }
    };
    put(json!({"format": 99, "entries": []})).await;
    assert_eq!(store.load("r").await.unwrap(), None);
    put(json!({"entries": []})).await;
    assert_eq!(store.load("r").await.unwrap(), None);
    put(json!({"format": 1, "entries": 5})).await;
    assert!(matches!(store.load("r").await, Err(StoreError::Corrupt(_))));
}

#[tokio::test]
async fn scopes_are_isolated() {
    let storage = MemoryStorage::new();
    let handle = |name: &str| {
        Arc::clone(
            storage
                .for_scope(&Scope::new(name).unwrap())
                .unwrap()
                .documents(),
        )
    };
    let alice = DocumentCatalogStore::new(handle("alice"));
    let bob = DocumentCatalogStore::new(handle("bob"));
    alice.save("r", &catalog()).await.unwrap();
    assert_eq!(bob.load("r").await.unwrap(), None);
}

#[test]
fn storage_errors_map_to_store_errors() {
    let serialization = StorageError::new(ErrorKind::Serialization, "bad json");
    assert!(matches!(
        storage_error(serialization),
        StoreError::Corrupt(_)
    ));
    let backend = StorageError::new(ErrorKind::Backend, "down");
    assert!(matches!(storage_error(backend), StoreError::Io(_)));
}
