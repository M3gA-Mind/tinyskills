//! User bundles as immutable blob generations selected by CAS-protected heads.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tinystoragedrivers_core::{BlobStore, DocumentStore, ErrorKind, Key, Precondition, Query, StorageError, Version, WriteOptions};
use tinyskills::{BundleDocument, BundleFuture, BundleRevision, BundleStore, BundleStoreError, BundleSummary, StoredBundle, UserBundle, BundleFile, BundleLimits, is_safe_segment};

use crate::BUNDLE_PREFIX;

/// The collection holding current user-bundle heads.
pub const BUNDLE_COLLECTION: &str = "skills.bundle_heads";
const BUNDLE_SCHEMA: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Head {
    revision: u64,
    tombstone: bool,
    primary: String,
    digest: String,
    generation: String,
}

/// A `BundleStore` backed by CAS documents and immutable blob generations.
#[derive(Clone)]
pub struct DocumentBundleStore {
    documents: Arc<dyn DocumentStore>,
    blobs: Arc<dyn BlobStore>,
}

impl std::fmt::Debug for DocumentBundleStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentBundleStore").finish_non_exhaustive()
    }
}

impl DocumentBundleStore {
    /// Bind the store to already-scoped storage handles supplied by the host.
    #[must_use]
    pub fn new(documents: Arc<dyn DocumentStore>, blobs: Arc<dyn BlobStore>) -> Self {
        Self { documents, blobs }
    }

    fn id(id: &str) -> Result<String, BundleStoreError> {
        if !is_safe_segment(id) { return Err(BundleStoreError::InvalidId); }
        Key::of([id]).map(|key| key.as_str().to_owned()).map_err(|_| BundleStoreError::InvalidId)
    }

    async fn head(&self, id: &str) -> Result<Option<(Head, Version)>, BundleStoreError> {
        let Some(row) = self.documents.get(BUNDLE_COLLECTION, id).await.map_err(backend)? else { return Ok(None); };
        let head = serde_json::from_value(row.doc).map_err(|_| BundleStoreError::Corrupt)?;
        Ok(Some((head, row.version)))
    }

    async fn files(&self, id: &str, head: &Head) -> Result<UserBundle, BundleStoreError> {
        let prefix = format!("{BUNDLE_PREFIX}/{id}/{}/", head.generation);
        let mut files = Vec::new();
        for meta in self.blobs.list(&prefix).await.map_err(backend)? {
            let blob = self.blobs.get(&meta.key).await.map_err(backend)?.ok_or(BundleStoreError::Corrupt)?;
            files.push(BundleFile { path: meta.key[prefix.len()..].to_owned(), bytes: blob.bytes });
        }
        files.sort_by(|a,b| a.path.cmp(&b.path));
        let primary = if head.primary == "skill" { BundleDocument::Skill } else if head.primary == "workflow" { BundleDocument::Workflow } else { return Err(BundleStoreError::Corrupt); };
        let primary_file = primary.file_name();
        let document = String::from_utf8(files.iter().find(|file| file.path == primary_file).ok_or(BundleStoreError::Corrupt)?.bytes.clone()).map_err(|_| BundleStoreError::Corrupt)?;
        let resources = files.into_iter().filter(|file| file.path != primary_file).collect();
        let content = UserBundle::new(primary, document, resources, BundleLimits::default()).map_err(|_| BundleStoreError::Corrupt)?;
        if content.digest() != head.digest { return Err(BundleStoreError::Corrupt); }
        Ok(content)
    }

    async fn write_content(&self, id: &str, revision: u64, content: &UserBundle) -> Result<String, BundleStoreError> {
        let generation = format!("{revision}-{}", content.digest());
        let prefix = format!("{BUNDLE_PREFIX}/{id}/{generation}");
        for file in content.files() {
            let key = format!("{prefix}/{}", file.path);
            match self.blobs.get(&key).await.map_err(backend)? {
                Some(existing) if existing.bytes == file.bytes => {},
                Some(_) => return Err(BundleStoreError::Corrupt),
                None => { self.blobs.put(&key, file.bytes.clone(), None).await.map_err(backend)?; }
            }
        }
        Ok(generation)
    }
}

fn backend(error: StorageError) -> BundleStoreError { BundleStoreError::Backend(Box::new(error)) }
fn conflict(error: StorageError, current: Option<BundleRevision>) -> BundleStoreError {
    if error.kind() == ErrorKind::Conflict || error.kind() == ErrorKind::AlreadyExists { BundleStoreError::Conflict { current } } else { backend(error) }
}
fn primary_name(primary: BundleDocument) -> &'static str { match primary { BundleDocument::Skill => "skill", BundleDocument::Workflow => "workflow" } }

impl BundleStore for DocumentBundleStore {
    fn read<'a>(&'a self, id: &'a str) -> BundleFuture<'a, Option<StoredBundle>> { Box::pin(async move {
        let id = Self::id(id)?;
        let Some((head, _)) = self.head(&id).await? else { return Ok(None); };
        if head.tombstone { return Ok(None); }
        let content = self.files(&id, &head).await?;
        Ok(Some(StoredBundle { revision: BundleRevision(head.revision), content }))
    }) }

    fn publish<'a>(&'a self, id: &'a str, expected: Option<BundleRevision>, content: &'a UserBundle) -> BundleFuture<'a, BundleRevision> { Box::pin(async move {
        let id = Self::id(id)?;
        let current = self.head(&id).await?;
        let current_visible = current.as_ref().filter(|(h,_)| !h.tombstone).map(|(h,_)| BundleRevision(h.revision));
        if current_visible != expected { return Err(BundleStoreError::Conflict { current: current_visible }); }
        let revision = current.as_ref().map_or(Some(1), |(head,_)| head.revision.checked_add(1)).ok_or(BundleStoreError::Corrupt)?;
        let generation = self.write_content(&id, revision, content).await?;
        let head = Head { revision, tombstone: false, primary: primary_name(content.primary()).into(), digest: content.digest(), generation };
        let precondition = current.as_ref().map_or(Precondition::Absent, |(_,version)| Precondition::Version(*version));
        self.documents.put_with(BUNDLE_COLLECTION, &id, serde_json::to_value(head).map_err(|e| BundleStoreError::Backend(Box::new(e)))?, precondition, WriteOptions::new().schema(BUNDLE_SCHEMA)).await.map_err(|e| conflict(e, current_visible))?;
        Ok(BundleRevision(revision))
    }) }

    fn list<'a>(&'a self, after: Option<&'a str>, limit: usize) -> BundleFuture<'a, Vec<BundleSummary>> { Box::pin(async move {
        if let Some(cursor) = after { Self::id(cursor)?; }
        let mut query = Query::all();
        let mut output = Vec::new();
        loop {
            let page = self.documents.query(BUNDLE_COLLECTION, &query).await.map_err(backend)?;
            for row in page.items {
                if after.is_some_and(|cursor| row.id.as_str() <= cursor) { continue; }
                let head: Head = serde_json::from_value(row.doc).map_err(|_| BundleStoreError::Corrupt)?;
                if head.tombstone { continue; }
                output.push(BundleSummary { id: row.id, revision: BundleRevision(head.revision), primary: if head.primary == "skill" { BundleDocument::Skill } else if head.primary == "workflow" { BundleDocument::Workflow } else { return Err(BundleStoreError::Corrupt); }, digest: head.digest });
                if output.len() >= limit { return Ok(output); }
            }
            match page.next { Some(cursor) => query.cursor = Some(cursor), None => return Ok(output) }
        }
    }) }

    fn delete<'a>(&'a self, id: &'a str, expected: BundleRevision) -> BundleFuture<'a, BundleRevision> { Box::pin(async move {
        let id = Self::id(id)?;
        let Some((mut head, version)) = self.head(&id).await? else { return Err(BundleStoreError::Conflict { current: None }); };
        let current = (!head.tombstone).then_some(BundleRevision(head.revision));
        if current != Some(expected) { return Err(BundleStoreError::Conflict { current }); }
        let revision = head.revision.checked_add(1).ok_or(BundleStoreError::Corrupt)?;
        head.revision = revision;
        head.tombstone = true;
        self.documents.put_with(BUNDLE_COLLECTION, &id, serde_json::to_value(head).map_err(|e| BundleStoreError::Backend(Box::new(e)))?, Precondition::Version(version), WriteOptions::new().schema(BUNDLE_SCHEMA)).await.map_err(|e| conflict(e, current))?;
        Ok(BundleRevision(revision))
    }) }
}
