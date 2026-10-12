//! Persistence contract for user bundles, separate from fetched registry catalogs.
//!
//! Implementations bind authority at construction. A bundle ID and revision are
//! data selectors, never scope, identity or generation admission. This module
//! implements no backend, file fallback, schema bootstrap or module transport.

use std::{future::Future, pin::Pin, sync::Arc};

use crate::{BundleDocument, UserBundle};

/// An uninterpreted store-issued revision token, not an access capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BundleRevision(pub u64);

/// One committed visible bundle and its publication revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredBundle {
    /// Exact committed publication revision.
    pub revision: BundleRevision,
    /// Validated original content.
    pub content: UserBundle,
}

/// Metadata for one committed visible bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleSummary {
    /// Identifier within this already-bound store, never a scope or path.
    pub id: String,
    /// Publication revision clients must compare when editing or deleting.
    pub revision: BundleRevision,
    /// Which primary document consumers select.
    pub primary: BundleDocument,
    /// Versioned digest from [`UserBundle::digest`].
    pub digest: String,
}

/// Why a user-bundle persistence operation refused to complete.
///
/// Backend refusals are not absence and must not trigger file/default fallback.
/// Internal adapters preserve the original typed source; a wire adapter must
/// map safe error kinds instead of serializing potentially sensitive sources.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BundleStoreError {
    /// The store refused an invalid data identifier.
    #[error("invalid user bundle identifier")]
    InvalidId,
    /// The expected visible revision did not match at commit.
    #[error("user bundle revision conflict")]
    Conflict {
        /// Current visible revision, or absent/deleted.
        current: Option<BundleRevision>,
    },
    /// Committed bytes or digest metadata failed integrity validation.
    #[error("committed user bundle content failed integrity validation")]
    Corrupt,
    /// Original typed backend error, including admission/fencing refusal.
    #[error("user bundle backend refused the operation")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Future returned by an object-safe user-bundle store.
pub type BundleFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, BundleStoreError>> + Send + 'a>>;

/// Durable publication of user bundles under already-captured authority.
///
/// This is distinct from [`crate::CatalogStore`] (when `registry` is enabled),
/// which caches remotely fetched catalog entries. Every method, including an
/// absent read or a zero-limit list, must enforce the implementation's captured
/// authority. No method selects a tenant, backend, generation or local path.
/// Native adapters must refuse missing authority and preserve typed native
/// errors; they may not manufacture schema authority or silently choose files.
///
/// Publication makes the head visible only after immutable content is complete.
/// Conditional head changes are atomic; document/blob ports need not form one
/// transaction. Failed pre-commit work must leave no visible partial head.
/// Once the head commits, optional cleanup failure cannot report it rolled back.
/// Deleted heads may remain as versioned tombstones; referenced immutable bytes
/// must remain safe for concurrent readers. This trait supplies no garbage
/// collection, legacy import, webhook routing or run-log authority.
pub trait BundleStore: Send + Sync {
    /// Read one committed bundle, distinguishing missing from empty file bytes.
    ///
    /// # Errors
    /// Invalid ID, corrupt content, or the original backend/admission refusal.
    fn read<'a>(&'a self, id: &'a str) -> BundleFuture<'a, Option<StoredBundle>>;

    /// Publish original bytes only if the current visible revision matches.
    ///
    /// `None` expects absence or deletion. A supplied token must match the live
    /// head. Recreation after deletion must still compare the underlying
    /// tombstone atomically. A successful result is the committed new revision.
    ///
    /// # Errors
    /// Invalid ID, conflict, or the original backend/admission refusal. There
    /// must be no new visible head when an error is returned.
    fn publish<'a>(
        &'a self,
        id: &'a str,
        expected: Option<BundleRevision>,
        content: &'a UserBundle,
    ) -> BundleFuture<'a, BundleRevision>;

    /// List committed heads in identifier order, strictly after `after`.
    ///
    /// Return at most `limit` items, excluding tombstones/staging records.
    /// The cursor is a data selector within this store, never authority.
    /// Concurrent publications may require callers to refresh a listing.
    ///
    /// # Errors
    /// Invalid cursor, corrupt metadata or the original backend/admission
    /// refusal, including when `limit` is zero.
    fn list<'a>(
        &'a self,
        after: Option<&'a str>,
        limit: usize,
    ) -> BundleFuture<'a, Vec<BundleSummary>>;

    /// Delete the live head only at its exact expected revision.
    ///
    /// Return the committed tombstone revision. Deletion cannot remove bytes
    /// needed by concurrent readers. Never erase an import's original source.
    ///
    /// # Errors
    /// Invalid ID, conflict/absence, or the original backend/admission refusal.
    /// A returned error must leave the live head unchanged.
    fn delete<'a>(
        &'a self,
        id: &'a str,
        expected: BundleRevision,
    ) -> BundleFuture<'a, BundleRevision>;
}

impl<T: BundleStore + ?Sized> BundleStore for Arc<T> {
    fn read<'a>(&'a self, id: &'a str) -> BundleFuture<'a, Option<StoredBundle>> {
        (**self).read(id)
    }

    fn publish<'a>(
        &'a self,
        id: &'a str,
        expected: Option<BundleRevision>,
        content: &'a UserBundle,
    ) -> BundleFuture<'a, BundleRevision> {
        (**self).publish(id, expected, content)
    }

    fn list<'a>(
        &'a self,
        after: Option<&'a str>,
        limit: usize,
    ) -> BundleFuture<'a, Vec<BundleSummary>> {
        (**self).list(after, limit)
    }

    fn delete<'a>(
        &'a self,
        id: &'a str,
        expected: BundleRevision,
    ) -> BundleFuture<'a, BundleRevision> {
        (**self).delete(id, expected)
    }
}

#[cfg(test)]
#[path = "bundle_store_tests.rs"]
mod tests;
