#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};
use tinystoragedrivers_api::Storage;
use tinystoragedrivers_testkit::memory_storage;
use tinystoragedrivers_types::{CollectionSpec, Precondition, Scope, WriteOptions};

use super::*;
use crate::schema::{CATALOG_SCHEMA, CATALOG_V1_COLLECTION, MIGRATIONS_COLLECTION};
use tinyskills::{CatalogStore, Validators};

fn v1_body() -> Value {
    serde_json::from_str(include_str!("../fixtures/catalog-v0.5.1.json")).unwrap()
}

fn storage() -> Storage {
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

async fn seed_v1_with_expiry(storage: &Storage, id: &str, body: Value, expires_ms: i64) {
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
            WriteOptions::new().expires_ms(expires_ms),
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
    let storage = storage();
    seed_v1(&storage, "example", v1_body()).await;

    let owner = SkillsStorage::open(storage.clone()).await.unwrap();
    let loaded = owner.catalogs().load("example").await.unwrap().unwrap();
    assert_eq!(loaded.fetched_at, 1_792_581_600);
    assert_eq!(loaded.skipped, 2);
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].entry.id, "fixture-skill");
    assert_eq!(loaded.validators.etag.as_deref(), Some("\"fixture-v1\""));

    let current = storage
        .documents()
        .get(CATALOG_COLLECTION, "example")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.meta.schema, CATALOG_SCHEMA);
    assert_eq!(current.doc["fetched_ms"], json!(1_792_581_600_000i64));
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
async fn owner_open_is_idempotent_for_a_valid_legacy_catalog() {
    let storage = storage();
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
    // The receipt records the last complete scan, but cannot hide a row an
    // older writer inserted afterwards.
    seed_v1(&storage, "late", v1_body()).await;
    SkillsStorage::open(storage.clone()).await.unwrap();
    assert!(
        storage
            .documents()
            .get(CATALOG_V1_COLLECTION, "late")
            .await
            .unwrap()
            .is_none()
    );
    let receipt = storage
        .documents()
        .get(MIGRATIONS_COLLECTION, "catalogs_v2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.doc, json!({"version": 1, "complete": true}));
}

#[tokio::test]
async fn malformed_or_unknown_legacy_catalogs_fail_closed_and_preserve_source() {
    let mut extra = v1_body();
    extra["future_field"] = json!("preserve me");
    for (id, body) in [
        ("future", json!({"format": 9, "entries": []})),
        ("broken", json!({"format": 1, "entries": "x"})),
        ("extra", extra),
    ] {
        let storage = storage();
        seed_v1(&storage, id, body.clone()).await;
        assert!(matches!(
            SkillsStorage::open(storage.clone()).await,
            Err(SkillsStorageError::MigrationConflict)
        ));
        assert_eq!(
            storage
                .documents()
                .get(CATALOG_V1_COLLECTION, id)
                .await
                .unwrap()
                .unwrap()
                .doc,
            body,
            "failed migration must keep the only copy of legacy data"
        );
        assert!(
            storage
                .documents()
                .get(CATALOG_COLLECTION, id)
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn a_conflicting_but_incomplete_current_catalog_does_not_delete_legacy_data() {
    let storage = storage();
    let source = v1_body();
    seed_v1(&storage, "example", source.clone()).await;
    storage
        .ensure_collection(&CollectionSpec::new(CATALOG_COLLECTION))
        .await
        .unwrap();
    storage
        .documents()
        .put_with(
            CATALOG_COLLECTION,
            "example",
            json!({}),
            Precondition::None,
            WriteOptions::new().schema(CATALOG_SCHEMA),
        )
        .await
        .unwrap();

    assert!(matches!(
        SkillsStorage::open(storage.clone()).await,
        Err(SkillsStorageError::MigrationConflict)
    ));
    assert_eq!(
        storage
            .documents()
            .get(CATALOG_V1_COLLECTION, "example")
            .await
            .unwrap()
            .unwrap()
            .doc,
        source
    );
}

#[tokio::test]
async fn an_unknown_owner_migration_receipt_fails_closed() {
    let storage = storage();
    storage
        .ensure_collection(&CollectionSpec::new(MIGRATIONS_COLLECTION))
        .await
        .unwrap();
    storage
        .documents()
        .put_with(
            MIGRATIONS_COLLECTION,
            "catalogs_v2",
            json!({"version": 99, "complete": true}),
            Precondition::None,
            WriteOptions::new(),
        )
        .await
        .unwrap();

    assert!(matches!(
        SkillsStorage::open(storage).await,
        Err(SkillsStorageError::MigrationConflict)
    ));
}

#[tokio::test]
async fn owner_open_preserves_catalog_expiry_metadata() {
    let storage = storage();
    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let expires_ms = now_ms + 60_000;
    seed_v1_with_expiry(&storage, "expiring", v1_body(), expires_ms).await;

    SkillsStorage::open(storage.clone()).await.unwrap();

    let migrated = storage
        .documents()
        .get(CATALOG_COLLECTION, "expiring")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(migrated.meta.expires_ms, Some(expires_ms));
    assert!(
        storage
            .documents()
            .get(CATALOG_V1_COLLECTION, "expiring")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn owner_open_migrates_all_pages_while_removing_source_rows() {
    let storage = storage();
    for index in 0..130 {
        seed_v1(&storage, &format!("registry-{index:03}"), v1_body()).await;
    }

    let owner = SkillsStorage::open(storage.clone()).await.unwrap();

    for index in 0..130 {
        let id = format!("registry-{index:03}");
        assert_eq!(
            owner
                .catalogs()
                .load(&id)
                .await
                .unwrap()
                .unwrap()
                .fetched_at,
            1_792_581_600
        );
        assert!(
            storage
                .documents()
                .get(CATALOG_V1_COLLECTION, &id)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        storage
            .documents()
            .get(MIGRATIONS_COLLECTION, "catalogs_v2")
            .await
            .unwrap()
            .unwrap()
            .doc,
        json!({"version": 1, "complete": true})
    );
}

#[tokio::test]
async fn current_catalog_store_still_round_trips_after_owner_open() {
    let storage = storage();
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
