#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;
use tinystoragedrivers_api::{DocumentStore, StorageBackend};
use tinystoragedrivers_testkit::{memory_backend, memory_storage};
use tinystoragedrivers_types::{ErrorKind, Precondition, Scope, StorageError, WriteOptions};

use super::*;
use crate::SkillsStorage;
use tinyskills::{RegistryEntry, Validators};

fn catalog() -> StoredCatalog {
    let mut entry = RegistryEntry::default();
    entry.entry.id = "a".to_owned();
    entry.entry.name = "a".to_owned();
    let mut validators = Validators::default();
    validators.etag = Some("\"e\"".to_owned());
    StoredCatalog::new(vec![entry], 42, validators, 3)
}

async fn docs(scope: &str) -> Arc<dyn DocumentStore> {
    let storage = memory_storage(&Scope::new(scope).unwrap());
    SkillsStorage::open(storage.clone()).await.unwrap();
    Arc::clone(storage.documents())
}

#[tokio::test]
async fn round_trips_and_replaces() {
    let store = DocumentCatalogStore::new(docs("alice").await);
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
async fn stores_the_v2_body_under_the_catalog_collection() {
    let documents = docs("alice").await;
    let store = DocumentCatalogStore::new(Arc::clone(&documents));
    store.save("example", &catalog()).await.unwrap();
    let found = documents
        .get("skills.catalogs", "example")
        .await
        .unwrap()
        .unwrap();
    let body = found.doc.as_object().unwrap();
    assert_eq!(body["fetched_ms"], json!(42_000));
    assert_eq!(body["skipped"], json!(3));
    assert!(body.contains_key("entries") && body.contains_key("validators"));
    assert!(!body.contains_key("format") && !body.contains_key("fetched_at"));
    assert_eq!(found.meta.schema, 2);
}

#[tokio::test]
async fn shares_state_across_clones_and_arc() {
    let store = DocumentCatalogStore::new(docs("alice").await);
    let copy = store.clone();
    store.save("r", &catalog()).await.unwrap();
    assert_eq!(copy.load("r").await.unwrap(), Some(catalog()));
    let shared: Arc<dyn CatalogStore> = Arc::new(copy);
    assert_eq!(shared.load("r").await.unwrap(), Some(catalog()));
}

#[tokio::test]
async fn refuses_unsafe_ids() {
    let store = DocumentCatalogStore::new(docs("alice").await);
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
async fn other_schemas_read_as_absent_and_bad_documents_as_corrupt() {
    let documents = docs("alice").await;
    let store = DocumentCatalogStore::new(Arc::clone(&documents));
    let put = |doc: serde_json::Value, schema: u32| {
        let documents = Arc::clone(&documents);
        async move {
            documents
                .put_with(
                    "skills.catalogs",
                    "r",
                    doc,
                    Precondition::None,
                    WriteOptions::new().schema(schema),
                )
                .await
                .unwrap();
        }
    };
    put(json!({"entries": []}), 99).await;
    assert_eq!(store.load("r").await.unwrap(), None);
    put(json!({"entries": []}), 0).await;
    assert_eq!(store.load("r").await.unwrap(), None);
    put(json!({"entries": 5}), 2).await;
    assert!(matches!(store.load("r").await, Err(StoreError::Corrupt(_))));
}

#[tokio::test]
async fn scopes_are_isolated() {
    let backend = memory_backend();
    let alice_storage = backend.for_scope(&Scope::new("alice").unwrap()).unwrap();
    let bob_storage = backend.for_scope(&Scope::new("bob").unwrap()).unwrap();
    SkillsStorage::open(alice_storage.clone()).await.unwrap();
    SkillsStorage::open(bob_storage.clone()).await.unwrap();
    let alice = DocumentCatalogStore::new(Arc::clone(alice_storage.documents()));
    let bob = DocumentCatalogStore::new(Arc::clone(bob_storage.documents()));
    alice.save("r", &catalog()).await.unwrap();
    assert_eq!(bob.load("r").await.unwrap(), None);
}

#[test]
fn storage_errors_map_to_store_errors() {
    let serialization = StorageError::new(ErrorKind::Serialization, "bad json");
    assert!(matches!(
        storage_error(&serialization),
        StoreError::Corrupt(_)
    ));
    let backend = StorageError::new(ErrorKind::Backend, "down");
    assert!(matches!(storage_error(&backend), StoreError::Io(_)));
}
