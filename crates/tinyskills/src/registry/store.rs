//! Persistence of fetched catalogs, and the clock that ages them.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::contract::{RegistryEntry, Validators};
use super::error::StoreError;
use super::transport::BoxFuture;

/// A persisted catalog for one registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct StoredCatalog {
    /// The storage format; a different value is read as absent.
    pub format: u32,
    /// The entries, in catalog order.
    pub entries: Vec<RegistryEntry>,
    /// Unix seconds of the fetch.
    pub fetched_at: u64,
    /// Validators for a conditional refresh.
    pub validators: Validators,
    /// Upstream items the load dropped.
    pub skipped: usize,
}

impl StoredCatalog {
    /// The storage format this crate writes.
    pub const FORMAT: u32 = 1;

    /// A catalog in the current format.
    #[must_use]
    pub fn new(
        entries: Vec<RegistryEntry>,
        fetched_at: u64,
        validators: Validators,
        skipped: usize,
    ) -> Self {
        Self {
            format: Self::FORMAT,
            entries,
            fetched_at,
            validators,
            skipped,
        }
    }
}

impl Default for StoredCatalog {
    fn default() -> Self {
        Self::new(Vec::new(), 0, Validators::default(), 0)
    }
}

/// Where fetched catalogs persist between runs.
pub trait CatalogStore: Send + Sync {
    /// The stored catalog for `registry`, or `None`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if the catalog cannot be read due to
    /// I/O error, file format issues, or symlink safety violations.
    fn load<'a>(
        &'a self,
        registry: &'a str,
    ) -> BoxFuture<'a, Result<Option<StoredCatalog>, StoreError>>;

    /// Replace the stored catalog for `registry`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if the catalog cannot be written due to
    /// I/O error, directory creation failure, or symlink safety violations.
    fn save<'a>(
        &'a self,
        registry: &'a str,
        catalog: &'a StoredCatalog,
    ) -> BoxFuture<'a, Result<(), StoreError>>;
}

impl<T: CatalogStore + ?Sized> CatalogStore for std::sync::Arc<T> {
    fn load<'a>(
        &'a self,
        registry: &'a str,
    ) -> BoxFuture<'a, Result<Option<StoredCatalog>, StoreError>> {
        (**self).load(registry)
    }

    fn save<'a>(
        &'a self,
        registry: &'a str,
        catalog: &'a StoredCatalog,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        (**self).save(registry, catalog)
    }
}

/// An in-process store; nothing survives the process.
#[derive(Debug, Default)]
pub struct MemoryCatalogStore {
    catalogs: Mutex<HashMap<String, StoredCatalog>>,
}

impl MemoryCatalogStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl CatalogStore for MemoryCatalogStore {
    fn load<'a>(
        &'a self,
        registry: &'a str,
    ) -> BoxFuture<'a, Result<Option<StoredCatalog>, StoreError>> {
        let stored = self
            .catalogs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(registry)
            .cloned();
        Box::pin(async move { Ok(stored) })
    }

    fn save<'a>(
        &'a self,
        registry: &'a str,
        catalog: &'a StoredCatalog,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        self.catalogs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(registry.to_owned(), catalog.clone());
        Box::pin(async { Ok(()) })
    }
}

/// The registry's notion of now, for ages and cooldowns.
pub trait Clock: Send + Sync {
    /// The current time.
    fn now(&self) -> SystemTime;
}

impl<T: Clock + ?Sized> Clock for std::sync::Arc<T> {
    fn now(&self) -> SystemTime {
        (**self).now()
    }
}

/// The system wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
