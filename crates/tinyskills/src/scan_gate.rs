//! The supply-chain scan gate shared by catalog and URL installs.
//!
//! A fetched `SKILL.md` whose scan blocks, or a fetch that fails, is fetched
//! and scanned once more. A document that still blocks is not installed unless
//! the user acknowledged that exact document (by its digest); the caller gets
//! [`Gated::ScanBlocked`] instead of an install.

use std::future::Future;

use serde::Serialize;

use crate::registry::{RegistryDocument, RegistryError, RegistryErrorKind};
use crate::scan::{ScanCheck, Verdict};

/// Whether the user reviewed a blocking scan and chose to install anyway.
///
/// The acknowledgement names the digest of the document the user was shown and
/// covers that document only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanAcknowledgement {
    /// Nobody acknowledged the findings: a blocking scan refuses the install.
    Absent,
    /// The user acknowledged the findings of the document with this digest.
    ByUser {
        /// Digest of the acknowledged document.
        digest: String,
    },
}

impl ScanAcknowledgement {
    /// The acknowledgement a caller-supplied digest stands for; a missing or
    /// blank digest acknowledges nothing.
    #[must_use]
    pub fn from_user_digest(digest: Option<String>) -> Self {
        match digest.map(|digest| digest.trim().to_owned()) {
            Some(digest) if !digest.is_empty() => Self::ByUser { digest },
            _ => Self::Absent,
        }
    }

    /// Whether any acknowledgement was given.
    #[must_use]
    pub fn is_given(&self) -> bool {
        matches!(self, Self::ByUser { .. })
    }

    /// Whether this acknowledgement lets `document` install despite its scan.
    #[must_use]
    pub fn covers(&self, document: &RegistryDocument) -> bool {
        match self {
            Self::Absent => false,
            Self::ByUser { digest } => *digest == document.digest,
        }
    }
}

/// One scan finding, in operator-facing language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanFindingSummary {
    /// The check that fired.
    pub check: ScanCheck,
    /// Its verdict.
    pub verdict: Verdict,
    /// Where in the skill it fired, such as `the document body`.
    pub field: String,
    /// The finding as one readable line, without the offending value.
    pub message: String,
}

/// A document the scan blocked, returned instead of installing it.
#[derive(Debug, Clone, Serialize)]
pub struct ScanBlocked {
    /// The catalog entry id or URL the caller asked to install.
    pub target: String,
    /// The URL the document was fetched from, redacted.
    pub fetched_from: String,
    /// The install slug the document would have used.
    pub slug: String,
    /// The blocked document's digest; acknowledging it installs this document
    /// and no other.
    pub digest: String,
    /// Every finding of the scan.
    pub findings: Vec<ScanFindingSummary>,
    /// A one-line refusal naming the blocking findings.
    pub message: String,
}

impl ScanBlocked {
    /// Summarize the blocking scan of `document` fetched for `target`.
    #[must_use]
    pub fn of(target: &str, document: &RegistryDocument) -> Self {
        let findings: Vec<ScanFindingSummary> = document
            .scan
            .findings
            .iter()
            .map(|finding| ScanFindingSummary {
                check: finding.check,
                verdict: finding.verdict,
                field: finding.field.label(),
                message: finding.message(),
            })
            .collect();
        let blocking: Vec<&str> = findings
            .iter()
            .filter(|finding| finding.verdict == Verdict::Block)
            .map(|finding| finding.message.as_str())
            .collect();
        Self {
            target: target.to_owned(),
            fetched_from: document.fetched_from.clone(),
            slug: document.document.slug.clone(),
            digest: document.digest.clone(),
            message: format!(
                "The security scan blocked this skill and it was not installed: {}.",
                blocking.join("; ")
            ),
            findings,
        }
    }
}

/// What a gated install did: ran the install, or refused it on its scan.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Gated<T> {
    /// The install ran.
    Installed(T),
    /// The scan refused it.
    ScanBlocked(ScanBlocked),
}

impl<T> Gated<T> {
    /// The wire status: `installed` or `scan_blocked`.
    #[must_use]
    pub fn status(&self) -> &'static str {
        match self {
            Self::Installed(_) => "installed",
            Self::ScanBlocked(_) => "scan_blocked",
        }
    }

    /// The install, or `None` when the scan refused it.
    #[must_use]
    pub fn installed(self) -> Option<T> {
        match self {
            Self::Installed(installed) => Some(installed),
            Self::ScanBlocked(_) => None,
        }
    }
}

/// Whether a failed fetch is worth one more attempt. Refusals that depend only
/// on the request (an unknown id, an unsafe URL, a portal entry), an oversized
/// body and an upstream asking to back off are not.
#[must_use]
pub fn fetch_error_is_retryable(error: &RegistryError) -> bool {
    !matches!(
        error.kind(),
        RegistryErrorKind::NotFound
            | RegistryErrorKind::Ambiguous
            | RegistryErrorKind::UpstreamAmbiguous
            | RegistryErrorKind::NoDirectDownload
            | RegistryErrorKind::UnsafeUrl
            | RegistryErrorKind::UnknownRegistry
            | RegistryErrorKind::TooLarge
            | RegistryErrorKind::RateLimited
    )
}

/// Fetch and scan a document, retrying once when the scan blocks or the fetch
/// fails with a retryable error. A blocked document the user acknowledged by
/// its digest is taken as is; one whose digest differs is unacknowledged.
///
/// # Errors
///
/// The fetch's [`RegistryError`]: the first attempt's when it is not
/// retryable, otherwise the retry's.
pub async fn fetch_scanned<F, Fut>(
    acknowledgement: &ScanAcknowledgement,
    mut fetch: F,
) -> Result<RegistryDocument, RegistryError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<RegistryDocument, RegistryError>>,
{
    match fetch().await {
        Ok(document) if !document.is_blocked() || acknowledgement.covers(&document) => {
            return Ok(document);
        }
        Ok(_) => {}
        Err(error) if fetch_error_is_retryable(&error) => {}
        Err(error) => return Err(error),
    }
    fetch().await
}

/// Turn a scanned document into an install, or into a refusal when its scan
/// blocks and the user has not acknowledged this document's digest.
///
/// # Errors
///
/// Whatever `install` returns.
pub fn gate_install<T, E>(
    target: &str,
    acknowledgement: &ScanAcknowledgement,
    document: RegistryDocument,
    install: impl FnOnce(RegistryDocument) -> Result<T, E>,
) -> Result<Gated<T>, E> {
    if document.is_blocked() && !acknowledgement.covers(&document) {
        return Ok(Gated::ScanBlocked(ScanBlocked::of(target, &document)));
    }
    install(document).map(Gated::Installed)
}

#[cfg(test)]
#[path = "scan_gate_tests.rs"]
mod tests;
