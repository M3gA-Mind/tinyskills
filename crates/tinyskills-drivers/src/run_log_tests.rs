#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use tinystoragedrivers_testkit::memory_storage;
use tinystoragedrivers_types::Scope;

#[tokio::test]
async fn stream_store_preserves_run_order_footer_lookup_and_byte_slices() {
    let storage = memory_storage(&Scope::new("alice").unwrap());
    let owner = crate::SkillsStorage::open(storage).await.unwrap();
    let store = owner.run_logs();
    let stream = store
        .start(
            "demo",
            "run-identifier-123",
            "20261021T090000Z",
            "2026-10-21T09:00:00+00:00",
            &serde_json::json!({"input": "value"}),
            "prompt",
        )
        .await
        .unwrap();
    store.append_line(&stream, "step α").await.unwrap();
    store
        .finish(&stream, "DONE", 12, "2026-10-21T09:00:01+00:00", "answer")
        .await
        .unwrap();

    let runs = store.scan(Some("demo"), 10).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, "run-identifier-123");
    assert_eq!(runs[0].status, "DONE");
    assert_eq!(
        store.find("run-identifier-123").await.unwrap(),
        Some(stream.clone())
    );
    assert_eq!(store.find("missing").await.unwrap(), None);

    let first = store.read_slice(&stream, 0, 1).await.unwrap();
    assert_eq!(first.offset, 1);
    assert_eq!(first.bytes_read, 1);
    let tail = store.read_slice(&stream, first.offset, 4096).await.unwrap();
    assert!(tail.content.contains("step α"));
    assert!(tail.content.contains("status  : DONE"));
    assert!(tail.complete);
    assert!(tail.eof);
    assert!(store.delete(&stream).await.unwrap());
    assert!(!store.delete(&stream).await.unwrap());
}

#[tokio::test]
async fn stream_names_sanitize_path_separators_and_reject_empty_identifiers() {
    let storage = memory_storage(&Scope::new("alice").unwrap());
    let owner = crate::SkillsStorage::open(storage).await.unwrap();
    assert!(matches!(
        owner
            .run_logs()
            .start("", "run", "stamp", "now", &serde_json::json!({}), "prompt")
            .await,
        Err(RunLogStoreError::InvalidId)
    ));
    let stream = owner
        .run_logs()
        .start(
            "owner/workflow",
            "run id",
            "stamp",
            "now",
            &serde_json::json!({}),
            "prompt",
        )
        .await
        .unwrap();
    assert!(!stream.contains('/'));
}
