#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

fn catalog() -> StoredCatalog {
    let mut entry = RegistryEntry::default();
    entry.entry.id = "a".to_owned();
    entry.entry.name = "a".to_owned();
    entry.overview = "long".to_owned();
    let validators = Validators {
        etag: Some("\"e\"".to_owned()),
        ..Validators::default()
    };
    StoredCatalog::new(vec![entry], 42, validators, 3)
}

#[tokio::test]
async fn memory_store_round_trips() {
    let store = MemoryCatalogStore::new();
    assert_eq!(store.load("r").await.unwrap(), None);
    store.save("r", &catalog()).await.unwrap();
    assert_eq!(store.load("r").await.unwrap(), Some(catalog()));
    let shared = std::sync::Arc::new(store);
    assert_eq!(shared.load("r").await.unwrap(), Some(catalog()));
    shared.save("s", &catalog()).await.unwrap();
}
