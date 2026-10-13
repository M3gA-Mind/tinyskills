#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{document_digest, parse_flat, scan_skill, validate_fetched_document};

const BLOCKED: &str =
    "---\nname: url-poisoned\ndescription: A pasted skill.\n---\n\n# Steps\nRun\u{200b} it.\n";
const OTHER_BLOCKED: &str = "---\nname: url-poisoned\ndescription: A pasted skill.\n---\n\n# Steps\nRun\u{200b} it twice.\n";
const CLEAN: &str =
    "---\nname: url-poisoned\ndescription: A pasted skill.\n---\n\n# Steps\nRun it.\n";

fn doc(text: &str) -> RegistryDocument {
    let document = validate_fetched_document(text.as_bytes()).expect("valid document");
    let flat = parse_flat(&document.content).expect("flat");
    let scan = scan_skill(&flat.scan_document(), &[]);
    RegistryDocument {
        entry: None,
        digest: document_digest(&document.content),
        document,
        flat,
        scan,
        fetched_from: "https://example.com/SKILL.md".into(),
    }
}

fn acknowledge(digest: &str) -> ScanAcknowledgement {
    ScanAcknowledgement::ByUser {
        digest: digest.to_owned(),
    }
}

/// A fetch closure serving `texts` in order (the last repeats) and counting calls.
fn serving(
    texts: &'static [&'static str],
    calls: &Arc<AtomicUsize>,
) -> impl FnMut() -> std::future::Ready<Result<RegistryDocument, RegistryError>> {
    let calls = Arc::clone(calls);
    move || {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(doc(texts[n.min(texts.len() - 1)])))
    }
}

#[test]
fn only_a_non_blank_digest_acknowledges_findings() {
    assert_eq!(
        ScanAcknowledgement::from_user_digest(Some(" abc ".into())),
        acknowledge("abc")
    );
    assert_eq!(
        ScanAcknowledgement::from_user_digest(Some("  ".into())),
        ScanAcknowledgement::Absent
    );
    assert_eq!(
        ScanAcknowledgement::from_user_digest(None),
        ScanAcknowledgement::Absent
    );
    assert!(!ScanAcknowledgement::Absent.is_given());
    assert!(acknowledge("x").is_given());
}

#[test]
fn outages_and_bad_documents_are_retried_but_request_refusals_are_not() {
    for retryable in [
        RegistryError::Unavailable { status: 503 },
        RegistryError::Timeout {
            operation: "document",
            budget: std::time::Duration::from_secs(1),
        },
    ] {
        assert!(fetch_error_is_retryable(&retryable), "{retryable}");
    }
    for refused in [
        RegistryError::NotFound {
            id: "x".into(),
            closest: Vec::new(),
        },
        RegistryError::NoDirectDownload {
            name: "x".into(),
            source_url: None,
        },
        RegistryError::RateLimited { retry_after: None },
    ] {
        assert!(!fetch_error_is_retryable(&refused), "{refused}");
    }
}

async fn run_failing(error: fn() -> RegistryError) -> (usize, RegistryError) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let result = fetch_scanned(&ScanAcknowledgement::Absent, move || {
        counted.fetch_add(1, Ordering::SeqCst);
        let error = error();
        async move { Err::<RegistryDocument, _>(error) }
    })
    .await;
    (
        calls.load(Ordering::SeqCst),
        result.expect_err("every attempt fails"),
    )
}

#[tokio::test]
async fn a_failed_fetch_is_attempted_exactly_twice() {
    let (calls, error) = run_failing(|| RegistryError::Unavailable { status: 503 }).await;
    assert_eq!(calls, 2);
    assert_eq!(error.kind(), RegistryErrorKind::Unavailable);
}

#[tokio::test]
async fn a_refused_request_is_not_retried() {
    let (calls, error) = run_failing(|| RegistryError::NotFound {
        id: "x".into(),
        closest: Vec::new(),
    })
    .await;
    assert_eq!(calls, 1);
    assert_eq!(error.kind(), RegistryErrorKind::NotFound);
}

#[tokio::test]
async fn a_clean_document_is_fetched_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let got = fetch_scanned(&ScanAcknowledgement::Absent, serving(&[CLEAN], &calls))
        .await
        .expect("fetch");
    assert!(!got.is_blocked());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_blocked_document_is_refetched_once_and_may_come_back_clean() {
    let calls = Arc::new(AtomicUsize::new(0));
    let got = fetch_scanned(
        &ScanAcknowledgement::Absent,
        serving(&[BLOCKED, CLEAN], &calls),
    )
    .await
    .expect("fetch");
    assert!(!got.is_blocked());
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let calls = Arc::new(AtomicUsize::new(0));
    let got = fetch_scanned(&ScanAcknowledgement::Absent, serving(&[BLOCKED], &calls))
        .await
        .expect("fetch");
    assert!(
        got.is_blocked(),
        "a persistent block is returned, not hidden"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn an_acknowledged_digest_takes_the_blocked_document_without_a_retry() {
    let digest = doc(BLOCKED).digest;
    let calls = Arc::new(AtomicUsize::new(0));
    let got = fetch_scanned(&acknowledge(&digest), serving(&[BLOCKED], &calls))
        .await
        .expect("fetch");
    assert!(got.is_blocked());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_stale_acknowledgement_does_not_cover_a_changed_document() {
    let seen = doc(BLOCKED).digest;
    let calls = Arc::new(AtomicUsize::new(0));
    let got = fetch_scanned(&acknowledge(&seen), serving(&[OTHER_BLOCKED], &calls))
        .await
        .expect("fetch");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_ne!(got.digest, seen);
    assert!(!acknowledge(&seen).covers(&got));
}

#[test]
fn gate_refuses_a_blocked_document_until_its_digest_is_acknowledged() {
    let installs = AtomicUsize::new(0);
    let install = |d: RegistryDocument| {
        installs.fetch_add(1, Ordering::SeqCst);
        Ok::<_, String>(d.document.slug)
    };

    let refused =
        gate_install("entry", &ScanAcknowledgement::Absent, doc(BLOCKED), install).expect("gate");
    let Gated::ScanBlocked(blocked) = &refused else {
        panic!("expected scan_blocked, got {refused:?}");
    };
    assert_eq!(blocked.target, "entry");
    assert_eq!(blocked.slug, "url-poisoned");
    assert_eq!(blocked.findings[0].check, ScanCheck::InvisibleCodePoints);
    assert_eq!(installs.load(Ordering::SeqCst), 0);

    let stale = gate_install("entry", &acknowledge("other"), doc(BLOCKED), install).expect("gate");
    assert_eq!(stale.status(), "scan_blocked");
    assert_eq!(installs.load(Ordering::SeqCst), 0);

    let digest = doc(BLOCKED).digest;
    let ok = gate_install("entry", &acknowledge(&digest), doc(BLOCKED), install).expect("gate");
    assert_eq!(ok.installed().as_deref(), Some("url-poisoned"));

    let clean =
        gate_install("entry", &ScanAcknowledgement::Absent, doc(CLEAN), install).expect("gate");
    assert_eq!(clean.status(), "installed");
    assert_eq!(installs.load(Ordering::SeqCst), 2);
}

#[test]
fn install_errors_pass_through_the_gate() {
    let err = gate_install("t", &ScanAcknowledgement::Absent, doc(CLEAN), |_| {
        Err::<(), _>("disk full".to_owned())
    })
    .expect_err("error");
    assert_eq!(err, "disk full");
}

#[test]
fn outcomes_serialize_with_a_status_tag() {
    let installed = Gated::Installed(serde_json::json!({ "new_skills": ["x"] }));
    let value = serde_json::to_value(&installed).expect("json");
    assert_eq!(value["status"], "installed");
    assert_eq!(value["new_skills"][0], "x");

    let refused = gate_install("x", &ScanAcknowledgement::Absent, doc(BLOCKED), |_| {
        Ok::<(), String>(())
    })
    .expect("gate");
    let value = serde_json::to_value(&refused).expect("json");
    assert_eq!(value["status"], "scan_blocked");
    assert_eq!(value["findings"][0]["check"], "invisible_code_points");
    assert_eq!(value["findings"][0]["verdict"], "block");
    assert_eq!(value["digest"], doc(BLOCKED).digest);
}
