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
    fail_get: bool,
    fail_put: bool,
    fail_list: bool,
}

#[async_trait]
impl BlobStore for GatedBlobs {
    async fn put(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<BlobMeta, StorageError> {
        if self.fail_put {
            return Err(StorageError::backend("injected blob write failure"));
        }
        self.inner.put(key, bytes, content_type).await
    }
    async fn get(&self, key: &str) -> Result<Option<Blob>, StorageError> {
        if self.fail_get {
            return Err(StorageError::backend("injected blob read failure"));
        }
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
        if self.fail_list {
            return Err(StorageError::backend("injected blob list failure"));
        }
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

fn parts(storage: &MemoryStorage) -> (Arc<dyn DocumentStore>, Arc<dyn BlobStore>) {
    let scoped = storage.for_scope(&Scope::new("alice").unwrap()).unwrap();
    (Arc::clone(scoped.documents()), Arc::clone(scoped.blobs()))
}

async fn put_head(documents: &dyn DocumentStore, id: &str, head: serde_json::Value) {
    documents
        .put_with(
            BUNDLE_COLLECTION,
            id,
            head,
            Precondition::None,
            WriteOptions::new().schema(BUNDLE_SCHEMA),
        )
        .await
        .unwrap();
}

fn store_with_gate(storage: &MemoryStorage, gate: Arc<ReadGate>) -> DocumentBundleStore {
    let scoped = storage.for_scope(&Scope::new("alice").unwrap()).unwrap();
    DocumentBundleStore::new(
        Arc::clone(scoped.documents()),
        Arc::new(GatedBlobs {
            inner: Arc::clone(scoped.blobs()),
            gate,
            fail_get: false,
            fail_put: false,
            fail_list: false,
        }),
    )
}

fn store_with_blob_fault(
    storage: &MemoryStorage,
    fail_get: bool,
    fail_put: bool,
    fail_list: bool,
) -> DocumentBundleStore {
    let scoped = storage.for_scope(&Scope::new("alice").unwrap()).unwrap();
    DocumentBundleStore::new(
        Arc::clone(scoped.documents()),
        Arc::new(GatedBlobs {
            inner: Arc::clone(scoped.blobs()),
            gate: Arc::new(ReadGate::default()),
            fail_get,
            fail_put,
            fail_list,
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
async fn absent_reads_and_store_debug_are_stable() {
    let storage = prepared().await;
    let store = store(&storage, "alice");
    assert_eq!(store.read("missing").await.unwrap(), None);
    assert!(format!("{store:?}").contains("DocumentBundleStore"));
}

#[tokio::test]
async fn workflow_primary_round_trips_and_corrupt_digest_is_refused() {
    let storage = prepared().await;
    let (documents, blobs) = parts(&storage);
    let store = DocumentBundleStore::new(Arc::clone(&documents), Arc::clone(&blobs));
    let workflow = UserBundle::new(
        BundleDocument::Workflow,
        "workflow".into(),
        Vec::new(),
        BundleLimits::default(),
    )
    .unwrap();
    store.publish("flow", None, &workflow).await.unwrap();
    assert_eq!(store.read("flow").await.unwrap().unwrap().content, workflow);

    let head: Head = serde_json::from_value(
        documents
            .get(BUNDLE_COLLECTION, "flow")
            .await
            .unwrap()
            .unwrap()
            .doc,
    )
    .unwrap();
    let key = format!("{BUNDLE_PREFIX}/flow/{}/WORKFLOW.md", head.generation);
    blobs.put(&key, b"tampered".to_vec(), None).await.unwrap();
    assert!(matches!(
        store.read("flow").await,
        Err(BundleStoreError::Corrupt)
    ));
}

#[tokio::test]
async fn malformed_heads_and_unknown_primary_documents_are_corrupt() {
    let storage = prepared().await;
    let (documents, blobs) = parts(&storage);
    let store = DocumentBundleStore::new(Arc::clone(&documents), Arc::clone(&blobs));
    put_head(&*documents, "malformed", serde_json::json!({"revision": 1})).await;
    assert!(matches!(
        store.read("malformed").await,
        Err(BundleStoreError::Corrupt)
    ));
    assert!(matches!(
        store.list(None, 10).await,
        Err(BundleStoreError::Corrupt)
    ));

    put_head(&*documents, "primary", serde_json::json!({"revision":1,"tombstone":false,"primary":"unknown","digest":"x","generation":"g"})).await;
    assert!(matches!(
        store.read("primary").await,
        Err(BundleStoreError::Corrupt)
    ));
    assert!(matches!(
        store.list(None, 10).await,
        Err(BundleStoreError::Corrupt)
    ));
}

#[tokio::test]
async fn stale_and_missing_deletes_report_the_current_visible_revision() {
    let storage = prepared().await;
    let store = store(&storage, "alice");
    assert!(matches!(
        store.delete("missing", BundleRevision(1)).await,
        Err(BundleStoreError::Conflict { current: None })
    ));
    let current = store.publish("demo", None, &content("one")).await.unwrap();
    assert!(
        matches!(store.delete("demo", BundleRevision(9)).await, Err(BundleStoreError::Conflict { current: Some(revision) }) if revision == current)
    );
}

#[tokio::test]
async fn blob_backend_refusals_do_not_publish_heads_or_look_like_absence() {
    let storage = prepared().await;
    let failing_writer = store_with_blob_fault(&storage, false, true, false);
    assert!(matches!(
        failing_writer.publish("demo", None, &content("one")).await,
        Err(BundleStoreError::Backend(_))
    ));
    assert_eq!(store(&storage, "alice").read("demo").await.unwrap(), None);

    let writer = store(&storage, "alice");
    writer.publish("demo", None, &content("one")).await.unwrap();
    let failing_reader = store_with_blob_fault(&storage, false, false, true);
    assert!(matches!(
        failing_reader.read("demo").await,
        Err(BundleStoreError::Backend(_))
    ));
}

#[tokio::test]
async fn content_generations_are_reused_only_for_identical_bytes() {
    let storage = prepared().await;
    let store = store(&storage, "alice");
    let content = content("same");
    let generation = store.write_content("demo", 7, &content).await.unwrap();
    assert_eq!(
        store.write_content("demo", 7, &content).await.unwrap(),
        generation
    );
    let file = &content.files()[0];
    let key = format!("{BUNDLE_PREFIX}/demo/{generation}/{}", file.path);
    let scoped = storage.for_scope(&Scope::new("alice").unwrap()).unwrap();
    scoped
        .blobs()
        .put(&key, b"different".to_vec(), None)
        .await
        .unwrap();
    assert!(matches!(
        store.write_content("demo", 7, &content).await,
        Err(BundleStoreError::Corrupt)
    ));
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
