use super::*;
use async_trait::async_trait;
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tinystoragedrivers_core::{
    Blob, BlobMeta, BlobStore, Capabilities, MemoryStorage, Registry, Scope, StorageBackend,
    StorageError,
};
use tokio::sync::Notify;

#[derive(Debug, Default)]
struct ReadGate {
    armed: AtomicBool,
    started: Notify,
    release: Notify,
}

#[derive(Debug)]
struct GatedBlobs {
    inner: Arc<dyn BlobStore>,
    gate: Arc<ReadGate>,
}

#[async_trait]
impl BlobStore for GatedBlobs {
    async fn put(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<BlobMeta, StorageError> {
        self.inner.put(key, bytes, content_type).await
    }
    async fn get(&self, key: &str) -> Result<Option<Blob>, StorageError> {
        if key.starts_with("skills/bundles/demo/") && self.gate.armed.swap(false, Ordering::SeqCst)
        {
            self.gate.started.notify_one();
            self.gate.release.notified().await;
        }
        self.inner.get(key).await
    }
    async fn get_range(
        &self,
        key: &str,
        range: Range<u64>,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        self.inner.get_range(key, range).await
    }
    async fn head(&self, key: &str) -> Result<Option<BlobMeta>, StorageError> {
        self.inner.head(key).await
    }
    async fn delete(&self, key: &str) -> Result<bool, StorageError> {
        self.inner.delete(key).await
    }
    async fn list(&self, prefix: &str) -> Result<Vec<BlobMeta>, StorageError> {
        self.inner.list(prefix).await
    }
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
}

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

fn store_with_gate(storage: &MemoryStorage, gate: Arc<ReadGate>) -> DocumentBundleStore {
    let scoped = storage.for_scope(&Scope::new("alice").unwrap()).unwrap();
    DocumentBundleStore::new(
        Arc::clone(scoped.documents()),
        Arc::new(GatedBlobs {
            inner: Arc::clone(scoped.blobs()),
            gate,
        }),
    )
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
        matches!(left.as_ref().err().or(right.as_ref().err()), Some(BundleStoreError::Conflict { current: Some(rev) }) if *rev == BundleRevision(2))
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
    let gate = Arc::new(ReadGate {
        armed: AtomicBool::new(false),
        ..ReadGate::default()
    });
    let store = store_with_gate(&storage, Arc::clone(&gate));
    let first = store.publish("demo", None, &content("one")).await.unwrap();
    gate.armed.store(true, Ordering::SeqCst);
    let reading = {
        let store = store.clone();
        tokio::spawn(async move { store.read("demo").await })
    };
    gate.started.notified().await;
    store
        .publish("demo", Some(first), &content("two"))
        .await
        .unwrap();
    gate.release.notify_one();
    let old = reading.await.unwrap().unwrap().unwrap();
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

#[tokio::test]
async fn listing_pages_in_identifier_order_and_skips_tombstones() {
    let storage = prepared().await;
    let store = store(&storage, "alice");
    let a = store.publish("a", None, &content("a")).await.unwrap();
    let b = store.publish("b", None, &content("b")).await.unwrap();
    store.publish("c", None, &content("c")).await.unwrap();
    store.delete("b", b).await.unwrap();

    assert_eq!(store.list(None, 0).await.unwrap(), Vec::new());
    let first = store.list(None, 1).await.unwrap();
    assert_eq!(
        first.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["a"]
    );
    let next = store.list(Some("a"), 1).await.unwrap();
    assert_eq!(
        next.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["c"]
    );
    assert!(matches!(
        store.list(Some("x/unsafe"), 1).await,
        Err(BundleStoreError::InvalidId)
    ));
    assert!(matches!(
        store.read(&"x".repeat(513)).await,
        Err(BundleStoreError::InvalidId)
    ));
    assert_eq!(store.read("a").await.unwrap().unwrap().revision, a);
}
