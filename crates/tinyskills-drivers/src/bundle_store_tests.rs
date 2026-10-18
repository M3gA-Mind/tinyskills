#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use tinystoragedrivers_core::{MemoryStorage, Registry, Scope, StorageBackend};

async fn prepared() -> MemoryStorage {
    let storage = MemoryStorage::new();
    Registry::compose([crate::storage_schema()])
        .unwrap()
        .prepare(&storage)
        .await
        .unwrap();
    storage
}

fn store(storage: &MemoryStorage, scope: &str) -> DocumentBundleStore {
    let scoped = storage.for_scope(&Scope::new(scope).unwrap()).unwrap();
    DocumentBundleStore::new(Arc::clone(scoped.documents()), Arc::clone(scoped.blobs()))
}

fn content(text: &str) -> UserBundle {
    UserBundle::new(
        BundleDocument::Skill,
        text.to_owned(),
        vec![BundleFile {
            path: "empty.bin".into(),
            bytes: Vec::new(),
        }],
        BundleLimits::default(),
    )
    .unwrap()
}

#[tokio::test]
async fn compare_and_swap_conflict_preserves_the_winning_head() {
    let storage = prepared().await;
    let a = store(&storage, "alice");
    let b = store(&storage, "alice");
    let first = a.publish("demo", None, &content("one")).await.unwrap();
    let two = content("two");
    let three = content("three");
    let (left, right) = tokio::join!(
        a.publish("demo", Some(first), &two),
        b.publish("demo", Some(first), &three)
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert!(
        matches!(left.as_ref().err().or(right.as_ref().err()), Some(BundleStoreError::Conflict { current: Some(rev) }) if *rev == first)
    );
    let actual = a.read("demo").await.unwrap().unwrap();
    assert_eq!(actual.revision, BundleRevision(2));
    assert!(
        actual.content.digest() == content("two").digest()
            || actual.content.digest() == content("three").digest()
    );
}

#[tokio::test]
async fn readers_keep_their_generation_while_a_new_revision_is_published() {
    let storage = prepared().await;
    let store = store(&storage, "alice");
    let first = store.publish("demo", None, &content("one")).await.unwrap();
    let old = store.read("demo").await.unwrap().unwrap();
    store
        .publish("demo", Some(first), &content("two"))
        .await
        .unwrap();
    assert_eq!(old.content.file("SKILL.md"), Some(b"one".as_slice()));
    assert_eq!(
        store
            .read("demo")
            .await
            .unwrap()
            .unwrap()
            .content
            .file("SKILL.md"),
        Some(b"two".as_slice())
    );
}

#[tokio::test]
async fn delete_commits_a_tombstone_and_allows_cas_recreation() {
    let storage = prepared().await;
    let store = store(&storage, "alice");
    let revision = store.publish("demo", None, &content("one")).await.unwrap();
    let deleted = store.delete("demo", revision).await.unwrap();
    assert_eq!(deleted, BundleRevision(2));
    assert_eq!(store.read("demo").await.unwrap(), None);
    assert_eq!(store.list(None, 10).await.unwrap(), Vec::new());
    let recreated = store.publish("demo", None, &content("two")).await.unwrap();
    assert_eq!(recreated, BundleRevision(3));
    assert_eq!(
        store
            .read("demo")
            .await
            .unwrap()
            .unwrap()
            .content
            .file("SKILL.md"),
        Some(b"two".as_slice())
    );
}
