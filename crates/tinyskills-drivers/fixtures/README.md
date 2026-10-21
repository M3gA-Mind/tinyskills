# Legacy catalog writer fixture

`catalog-v0.5.1.json` was emitted by the public `DocumentCatalogStore::save`
implementation in the released TinySkills `v0.5.1` source at commit
`f46cd334814f6062725c92ae8567034ebb69b930`. That release's unmodified
`Cargo.lock` resolves `tinystoragedrivers-core` v0.11.0. The fixture is the
actual document body read back from the old writer's in-memory document store;
the migration tests import it through `SkillsStorage::open` and read it through
the current catalog API.

The checked-in artifact SHA-256 is
`1b8d4d6486f269fbac6a81e93dd658dd7cacabbed7b4d44f4e79be9127417ac9`.

The archived writer source is retained outside build output at
`<superproject>/.epic-repositories/legacy-tinyskills-v051-clean` for review. To
regenerate it from a clean checkout without changing the historical source or
lockfile:

1. `git archive v0.5.1 | tar -x -C .epic-repositories/legacy-tinyskills-v051-clean`.
2. Copy `crates/tinyskills-drivers/fixtures/generate_catalog_v0_5_1.rs` to
   `.epic-repositories/legacy-tinyskills-v051-clean/crates/tinyskills-drivers/examples/write_catalog_fixture.rs`.
   Cargo auto-discovers examples under the crate, so no manifest edit is needed.
3. Run `cargo run --locked -p tinyskills-drivers --example write_catalog_fixture`
   from `.epic-repositories/legacy-tinyskills-v051-clean`, redirecting stdout to the
   checked-in fixture path.

The only added file in the archived copy is this temporary generator. The
original tracked source, manifest, and lockfile remain byte-identical to the
tag. The generator uses the released store API to write and read the row; it
does not construct the serialized catalog body itself.
