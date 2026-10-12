# User bundles behind an admitted module contract

Status: source-backed design, not an implemented cloud store or host migration.
Baseline: TinySkills `cf64fbc`; its pinned TinyBus is `356bb24f`.
Canonical runtime separately inspected at `9062dd8` after a fresh fetch.
Pinned-source limitations below are not attributed to current runtime main.

## Existing contracts and boundaries

- Workspace `Cargo.toml` has only `crates/tinyskills`. The former template bus
  and module were removed. Release CI packages a Rust crate, not a compiled
  module. A product must not link this implementation to work around the
  absent module contract.
- `registry/store.rs::CatalogStore` loads and saves `StoredCatalog`, which
  holds fetched registry entries, validators and refresh timestamps. It has
  no user bundle, revision, conditional mutation or deletion operation.
  Extend through a separate `BundleStore` rather than reinterpret fetched
  catalog JSON or break existing catalog implementors.
- `archive.rs::SkillArchive` already preserves the document and resource
  bytes. `read_skill_archive` validates archive structure and limits.
  `materialize.rs::MaterializeSource` currently supports an open directory or
  a single document; a persisted multi-file bundle needs a validated file
  source. `materialize_tree` resolves relative to capability directory handles.
- TinyBus message `Header.sender` is broker-stamped. However the pinned
  `service::Interface::call` receives only member and positional JSON args;
  connection dispatch does not expose authenticated caller identity to an
  ordinary typed interface.
- Pinned `broker.rs::bus_method` dispatches `module_method(member, body)`
  without passing the authenticated connection. `module_method` handles
  LoadModule and ReinitializeModule. The sensitive flag restricts observation,
  not which peer may replace module configuration. Treating sensitive
  configuration as controller authentication would create authority.
- Native scoped storage is a Rust trait object and cannot be serialized
  through the compiled-module ABI. A scope string, document ID, database URL,
  epoch number or hash is not an admitted capability. Do not invent a host
  callback storage implementation or transmit raw pointers across the ABI.

## Current canonical runtime and remaining admission prerequisite

Canonical TinyBus `9062dd8` already provides `CallContext`,
`Interface::call_with_context`, default legacy forwarding and Arc forwarding.
The context has no serializable authority or public authenticated constructor.
Memory broker deliveries have private routing proof; module deliveries use the
trusted host's additive ABI routing callback. Unix/custom transports without
that proof stay unverified even after Hello. `setup_factory` state lifetimes
are also available. Consume these canonical APIs instead of inventing another
context hook. No runtime source was modified or tested by this audit.

Current canonical `broker.rs::bus_method` still calls
`module_method(member, body)` without authenticated caller identity. Therefore
caller context solves ordinary actor-method admission, but does not establish
controller authorization for replacement module configuration. Only open PR51
was present at inspection; it concerns shared CI, not controller admission.

Required remaining contract is trusted module-control admission, or an
explicit private-broker isolation contract with enforceable equivalent
invariants. Proposed sequence for owning runtime review:

1. Use existing `call_with_context` and reject unverified caller contexts.
   Bind authenticated sender to a trusted initialized actor; a sender names a
   connection, not a person or tenant. Ordinary positional arguments must
   never substitute for context.
2. Bind module configuration to a host-selected controller connection at
   broker installation. Enforce admission on load/reinitialize/enable/stop
   before callbacks or state mutation. Do not let an ordinary peer select or
   replace the controller. Preserve a clearly documented legacy mode for
   existing users; the cloud module requires the admitted mode.
3. Trusted setup creates an immutable actor record associated with the
   authenticated calling connection. Ordinary methods accept opaque actor
   handles, checked against that caller, never a profile to look up.
   Reconfiguration must not widen existing actor records; old handles retain
   their original grant or refuse. Disconnect revokes handles.
4. Each actor opens native records with the exact captured scope, generation,
   fence and explicit guard database. Require ExistingSchema and collection
   validation; bootstrap schema creation is a separate trusted operation.
   No registry refresh, scope derivation from bundle IDs or unfenced fallback.
   Wire fence/filter DTOs require a reviewed native encoding, bounded values
   and debug redaction; they remain controller input, not public method input.

Regression proof for the runtime prerequisite: two peers cannot configure
one another's module, caller identity cannot be forged in JSON, legacy
callbacks still work, retained actor A cannot inherit B after reinit, and
unauthorized configuration invokes no setup callback. A static manifest
capability alone is insufficient proof.

## TinySkills owner implementation after runtime admission exists

Add minimal `tinyskills-bus` contract and a compiled module implemented by
TinySkills. Host dependencies name only the bus contract. The contract exposes
publish/read/list/delete/materialize over actor handles and bundle IDs,
expected revisions and bounded content streams. It does not expose backend
URLs, arbitrary scopes, directory roots or schema creation through ordinary
requests. Request/response errors have stable kinds; internal native errors
retain their typed source, and wire refusals are never absence.

The module owns `BundleStore` implementation on native documents and blobs.
Its constructor receives already admitted handles internally. Existing
CatalogStore remains source-catalog caching. File and memory BundleStore
implementations may support standalone desktop consumers without authorizing
cloud fallback. Module release/manifest/checksum tests are part of this owner
slice; shipping only a Rust crate does not finish the bus contract.

### Durable model and publication

A bundle head is one versioned document, with slug, parsed metadata,
immutable manifest digest, revision and tombstone state. The manifest records
original relative file names, byte lengths and digests. Store document bytes
and resources as immutable content-addressed blobs with bounded names and
sizes; never use tenant metadata as the blob namespace.

Upload immutable bytes first using explicitly admitted staging authority,
then publish the manifest/head via native conditional document mutation on
captured fence and generation. Cross-port storage does not imply a global
transaction. Failed publication leaves unreachable bytes, never a visible
partial bundle. An acknowledged head commit remains success even if optional
cleanup fails. A composite driver that refuses fenced blob writes must not
silently obtain raw blob mutation authority: staging requires its own
explicit contract. Garbage collection must preserve concurrent readers and
cannot delete shared digest objects based solely on a lost head CAS.

Read missing versus an empty file distinctly; validate head and manifest
before fetching content, verify every digest and size. List only committed
heads. Delete CASes a tombstone before any garbage collection. Conflicting
edits preserve both callers' prior data; no last-writer-wins save API.

### Materialization and desktop import

Materialization consumes a committed immutable snapshot. Trusted module
configuration grants an owned local cache root; requests cannot name paths.
Build a new digest-addressed tree relative to open handles, refusing traversal,
links, duplicates and limit excess. The host receives a read-only view and
never authors it. Publishing a newer tree does not mutate an older session's
snapshot. Existing session frozen-prompt behavior stays a host concern.

Legacy import requires an explicit trusted source capability and destination
actor plus expected generation. Read source bytes without changing them;
record source identity/digest and imported heads in the destination scope with
conditional import receipts. Retrying the same receipt is idempotent;
changed bytes need explicit conflict handling. A global once-per-backend
marker, HOME discovery, caller-selected path or source name is not authority.
A retired destination generation cannot import or publish. Do not erase
source data after import. Desktop local mirror compatibility requires its own
positive fixture rather than treating absence/refusal as local selection.

## Execution and acceptance sequence

1. Update the TinySkills runtime pin to available canonical context/factory
   contracts, preserving history. Land the remaining controller-configuration
   admission contract in TinyBus or prove private broker isolation; publish
   its owner PR and consume only available canonical behavior.
2. Add the TinySkills bundle data/revision/error contract and archive/path
   validation regressions. No implicit import or storage URL selection.
3. Native store TDD on Memory and SQLite: independently opened A handles
   cross-read, B isolation, missing/empty, competing CAS, deletion tombstones,
   failed blob/read/digest, failed publication with no visible head, exact
   stale fence/generation refusals and retained source bytes.
4. Real opt-in loopback Mongo native conformance: independent actor handles,
   publication and rejection after takeover. Default tests use no remote
   services; skipped fixture is not acceptance evidence.
5. Module ABI tests call the compiled component through `tinyskills-bus`;
   interleave two caller actors, reconnect/reinitialize and verify authority
   lifetimes. Test malicious raw scope/profile/config fields cannot route.
6. Source-preserving desktop import and materialization tests use isolated
   source/cache directories, read-only consumer views and symlink adversaries.
7. Full feature-enabled/disabled tests, strict Clippy/build/docs, coverage,
   compiled-module release artifacts and manifest compatibility. Publish a
   concrete independently reviewable owner PR. Host subscription, webhook
   route documents, run-log streams and local bypass removal remain separate
   host adapters after canonical availability.

No cloud bundle behavior, runtime authorization, native generation protection,
source import or ProfileHost acceptance has been executed by this design.
