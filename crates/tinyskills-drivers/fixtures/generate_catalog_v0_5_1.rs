//! Emit one legacy catalog by calling the released public storage writer.

use std::sync::Arc;

use tinystoragedrivers_core::{MemoryStorage, Scope, StorageBackend};
use tinyskills::{CatalogStore, RegistryEntry, StoredCatalog, Validators};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let backend = MemoryStorage::new();
    let scoped = backend.for_scope(&Scope::new("fixture")?)?;
    let documents = Arc::clone(scoped.documents());
    let store = tinyskills_drivers::DocumentCatalogStore::new(documents.clone());

    let mut entry = RegistryEntry::default();
    entry.entry.id = "fixture-skill".into();
    entry.entry.name = "Fixture Skill".into();
    entry.entry.description = "Written by the released public storage adapter.".into();
    let mut validators = Validators::default();
    validators.etag = Some("\"fixture-v1\"".into());
    validators.last_modified = Some("Wed, 21 Oct 2026 09:00:00 GMT".into());
    let catalog = StoredCatalog::new(vec![entry], 1_792_581_600, validators, 2);

    store.save("fixture-registry", &catalog).await?;
    let row = documents
        .get("tinyskills_catalogs", "fixture-registry")
        .await?
        .expect("public writer created the catalog row");
    println!("{}", serde_json::to_string_pretty(&row.doc)?);
    Ok(())
}
