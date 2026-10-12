use super::*;
use std::sync::{Mutex, PoisonError};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Debug, thiserror::Error)]
#[error("private backend detail")]
struct TypedRefusal;

#[derive(Default)]
struct RecordingAdapter(Mutex<Vec<String>>);

impl BundleStore for RecordingAdapter {
    fn read<'a>(&'a self, id: &'a str) -> BundleFuture<'a, Option<StoredBundle>> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("read:{id}"));
        Box::pin(async { Err(BundleStoreError::Backend(Box::new(TypedRefusal))) })
    }
    fn publish<'a>(
        &'a self,
        id: &'a str,
        expected: Option<BundleRevision>,
        content: &'a UserBundle,
    ) -> BundleFuture<'a, BundleRevision> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("publish:{id}:{expected:?}:{}", content.digest()));
        Box::pin(async { Ok(BundleRevision(42)) })
    }
    fn list<'a>(
        &'a self,
        after: Option<&'a str>,
        limit: usize,
    ) -> BundleFuture<'a, Vec<BundleSummary>> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("list:{after:?}:{limit}"));
        Box::pin(async { Ok(Vec::new()) })
    }
    fn delete<'a>(
        &'a self,
        id: &'a str,
        expected: BundleRevision,
    ) -> BundleFuture<'a, BundleRevision> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("delete:{id}:{expected:?}"));
        Box::pin(async {
            Err(BundleStoreError::Conflict {
                current: Some(BundleRevision(43)),
            })
        })
    }
}

#[tokio::test]
async fn erased_arc_forwarding_preserves_selectors_content_revisions_and_typed_refusals()
-> TestResult {
    let original = Arc::new(RecordingAdapter::default());
    let erased: Arc<dyn BundleStore> = original.clone();
    let content = UserBundle::new(
        BundleDocument::Workflow,
        "source".into(),
        Vec::new(),
        crate::BundleLimits::default(),
    )?;
    // This is forwarding-contract evidence, not a backend implementation or
    // proof that the recording adapter enforces native authority/transactions.
    let error = BundleStore::read(&erased, "chosen")
        .await
        .err()
        .ok_or("expected store refusal")?;
    assert!(!error.to_string().contains("private"));
    assert!(
        std::error::Error::source(&error)
            .ok_or("missing typed source")?
            .is::<TypedRefusal>()
    );
    assert_eq!(
        BundleStore::publish(&erased, "chosen", Some(BundleRevision(41)), &content).await?,
        BundleRevision(42)
    );
    assert!(
        BundleStore::list(&erased, Some("cursor"), 0)
            .await?
            .is_empty()
    );
    let error = BundleStore::delete(&erased, "chosen", BundleRevision(41))
        .await
        .err()
        .ok_or("expected store refusal")?;
    assert!(matches!(
        error,
        BundleStoreError::Conflict {
            current: Some(BundleRevision(43))
        }
    ));
    assert_eq!(
        *original.0.lock().unwrap_or_else(PoisonError::into_inner),
        vec![
            "read:chosen".to_owned(),
            format!(
                "publish:chosen:Some(BundleRevision(41)):{}",
                content.digest()
            ),
            "list:Some(\"cursor\"):0".to_owned(),
            "delete:chosen:BundleRevision(41)".to_owned()
        ]
    );
    Ok(())
}
