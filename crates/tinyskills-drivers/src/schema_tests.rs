#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};
use tinystoragedrivers_api::Storage;
use tinystoragedrivers_testkit::memory_storage;
use tinystoragedrivers_types::{CollectionSpec, Precondition, Scope, WriteOptions};

use super::*;
use crate::schema::{CATALOG_SCHEMA, CATALOG_V1_COLLECTION};
use tinyskills::{CatalogStore, Validators};

fn v1_body() -> Value {
    json!({
        "format": 1,
        "entries": [{
            "entry": {
                "id": "a", "name": "a", "description": "", "source": "", "category": "",
                "author": null, "version": null, "tags": [], "platforms": [],
                "download_url": "", "source_url": null, "docs_path": null,
                "commands": [], "env_vars": [], "license": null
            },
            "overview": "o",
            "category_label": null, "fixed_category": null, "install_identifier": null
        }],
        "fetched_at": 42,
        "validators": {"etag": "\"e\"", "last_modified": null},
        "skipped": 3
    })
}

async fn storage() -> Storage {
    memory_storage(&Scope::new("alice").unwrap())
}

async fn seed_v1(storage: &Storage, id: &str, body: Value) {
    storage
        .ensure_collection(&CollectionSpec::new(CATALOG_V1_COLLECTION))
        .await
        .unwrap();
    storage
        .documents()
        .put_with(
            CATALOG_V1_COLLECTION,
            id,
            body,
            Precondition::None,
            WriteOptions::new(),
        )
        .await
        .unwrap();
}

#[test]
fn declarations_are_pure_and_name_only_the_skills_owner() {
    let names: Vec<_> = collections().into_iter().map(|spec| spec.name).collect();
    assert_eq!(
        names,
        [CATALOG_COLLECTION, BUNDLE_COLLECTION, MIGRATIONS_COLLECTION]
    );
    assert_eq!(blob_prefixes(), [BUNDLE_PREFIX, BUNDLE_GENERATION_PREFIX]);
    assert_eq!(CATALOG_COLLECTION, "skills.catalogs");
    assert_eq!(CATALOG_SCHEMA, 2);
}

#[tokio::test]
async fn owner_open_ensures_collections_and_migrates_legacy_catalogs() {
    let storage = storage().await;
    seed_v1(&storage, "example", v1_body()).await;

    let owner = SkillsStorage::open(storage.clone()).await.unwrap();
    let loaded = owner.catalogs().load("example").await.unwrap().unwrap();
    assert_eq!(loaded.fetched_at, 42);
    assert_eq!(loaded.skipped, 3);
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.validators.etag.as_deref(), Some("\"e\""));

    let current = storage
        .documents()
        .get(CATALOG_COLLECTION, "example")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.meta.schema, CATALOG_SCHEMA);
    assert_eq!(current.doc["fetched_ms"], json!(42_000));
    assert!(current.doc.get("format").is_none());
    assert!(
        storage
            .documents()
            .get(CATALOG_V1_COLLECTION, "example")
            .await
            .unwrap()
            .is_none()
    );
    for spec in collections() {
        storage
            .documents()
            .validate_collection_spec(&spec)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn owner_open_is_idempotent_and_drops_unreadable_legacy_cache_rows() {
    let storage = storage().await;
    seed_v1(&storage, "future", json!({"format": 9, "entries": []})).await;
    seed_v1(&storage, "broken", json!({"format": 1, "entries": "x"})).await;
    seed_v1(&storage, "good", v1_body()).await;

    let first = SkillsStorage::open(storage.clone()).await.unwrap();
    let saved = storage
        .documents()
        .get(CATALOG_COLLECTION, "good")
        .await
        .unwrap();
    let second = SkillsStorage::open(storage.clone()).await.unwrap();
    assert_eq!(
        storage
            .documents()
            .get(CATALOG_COLLECTION, "good")
            .await
            .unwrap(),
        saved
    );
    assert_eq!(
        first.catalogs().load("good").await.unwrap(),
        second.catalogs().load("good").await.unwrap()
    );
    for id in ["future", "broken"] {
        assert!(
            storage
                .documents()
                .get(CATALOG_COLLECTION, id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            storage
                .documents()
                .get(CATALOG_V1_COLLECTION, id)
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn current_catalog_store_still_round_trips_after_owner_open() {
    let storage = storage().await;
    let owner = SkillsStorage::open(storage).await.unwrap();
    let mut validators = Validators::default();
    validators.etag = Some("etag".to_owned());
    let catalog = tinyskills::StoredCatalog::new(Vec::new(), 42, validators, 3);
    owner.catalogs().save("registry", &catalog).await.unwrap();
    assert_eq!(
        owner.catalogs().load("registry").await.unwrap(),
        Some(catalog)
    );
}
