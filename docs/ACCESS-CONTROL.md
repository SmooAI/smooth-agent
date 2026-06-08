# Document-level access control

Onyx syncs per-connector permissions and filters retrieval by user entitlement;
before this, smooth-operator filtered knowledge by `organizationId` only.
This is the within-org **document-level** layer (Onyx-gap **G3**, the
highest-severity gap in [ONYX-TESTING-GAP-ANALYSIS.md](ONYX-TESTING-GAP-ANALYSIS.md)):
even inside one organization, a document may be restricted to specific users or
groups, and a retrieval must only ever return documents the requester is
entitled to read.

Org isolation is unchanged and still happens upstream (the Postgres knowledge
base filters on `organizationId`, DynamoDB scopes per-org indexes). Access
control is **additive on top** of org isolation.

## Where enforcement lives — our layer, not the engine

smooth-operator-core's `KnowledgeBase` trait is upstream and read-only to this repo.
Two facts force enforcement into our layer:

1. `KnowledgeBase::query` returns a `KnowledgeResult` that carries only
   `document_id` / `chunk` / `score` / `source` — **not** the stored metadata.
2. The in-memory backend drops document metadata on ingest entirely; the
   Postgres backend stores it but doesn't return it from `query`.

So we cannot read an ACL back out of a query result. Instead, the
`AclKnowledgeStore` (in `smooth-operator`, `src/access_control.rs`)
wraps any inner `KnowledgeBase` and:

- **records the document → ACL mapping at ingest** into a side table it owns
  (parsed from the document metadata), forwarding the document unchanged to the
  inner backend; then
- **filters at read**: a per-requester reader over-fetches from the inner
  backend, looks each result's ACL up in the side table, and drops any the
  requester cannot access.

This is **backend-agnostic**: the same wrapper sits in front of the in-memory,
Postgres, or DynamoDB knowledge base identically — the post-filter runs in our
layer, after the backend's own org-scoped query. In-memory and Postgres paths
are exercised by tests; DynamoDB follows the same post-filter (no per-backend
ACL code).

## The model

### `DocAcl` — the document's allow-list

```rust
pub struct DocAcl {
    pub public: bool,        // visible to anyone reaching it
    pub users:  Vec<String>, // user ids explicitly allowed
    pub groups: Vec<String>, // group ids explicitly allowed
}
```

A document is **visible** to a requester when **any** of:

- `public == true`, or
- the requester's `user_id` ∈ `users`, or
- any of the requester's `groups` ∈ `groups`.

`DocAcl` serializes to JSON and rides in the document metadata under the key
`acl_v2` (`DocAcl::ACL_METADATA_KEY`). `DocAcl::attach_to(doc)` stamps it on;
`DocAcl::from_metadata(&doc.metadata)` reads it back. A **malformed** stamp
parses as "absent" (falls back to the default) so a corrupt value can't silently
lock or unlock a document.

### `AccessContext` — the requester's identity

```rust
pub struct AccessContext {
    pub user_id: Option<String>, // None for anonymous / system
    pub groups:  Vec<String>,
}

ctx.can_access(&acl) -> bool   // the gate
```

Built upstream from the authenticated user + their resolved group memberships.

## No-ACL default semantics — **no-acl ⇒ org-public**

This is the load-bearing backward-compatibility choice:

- A document ingested **without** an ACL (the legacy / existing-seed path) has
  **no entry** in the side table and is treated as **org-public** — visible to
  anyone whose query reaches it. Org isolation already happened upstream. This
  keeps all existing seeded knowledge retrievable; ACLs are strictly additive,
  opting a document *into* restriction.
- An **explicit** `DocAcl::default()` (`public: false`, empty `users`/`groups`)
  is the opposite: a fully-locked document only its listed users/groups can read.

So "no ACL recorded at all" (org-public) and "an empty ACL recorded"
(fully-locked) are deliberately different states.

## Over-fetch then filter

Filtering happens **after** the inner backend ranks results, so naively asking
the backend for `K` and then dropping the inaccessible ones would under-fill the
top-`K`. The reader instead **over-fetches**: it queries the inner backend for
`max(K * 5, 20)` candidates, filters by `can_access`, and truncates to `K`. So
the post-filter top-`K` stays full whenever enough accessible documents exist.
This mirrors the over-fetch the Postgres backend already does to feed RRF fusion.

## Wiring it into retrieval

`AccessContext` is threaded into **both** retrieval paths so neither can leak:

- **`KnowledgeChatRuntime::with_access_control(store, context)`** — when set,
  every turn reads knowledge through an `AccessContext`-bound reader. That one
  reader feeds both (a) the engine's auto-injected `[Relevant knowledge]`
  context (`AgentConfig::with_knowledge`) and (b) the `knowledge_search` tool —
  so the model never sees a restricted snippet through either path. Without it,
  the runtime reads the raw `storage.knowledge()` exactly as before
  (backward-compatible).
- **`KnowledgeSearchTool::with_access_control(&store, context)`** — builds the
  tool directly bound to a requester, for callers wiring tools by hand.

Ingestion stamps ACLs automatically: the pipeline's `RawDocument.acl` labels are
written as a `DocAcl` (interpreted as **group** entitlements — the common
connector-permission shape) under `acl_v2`, in addition to the legacy
comma-joined `acl` field kept for debug visibility. See
[INGESTION.md](INGESTION.md).

## Tests

`smooth-operator/tests/access_control.rs`:

- **The cross-user leak test** (written first, failed before enforcement
  existed): three docs share a query term — `doc-a` (alice-only), `doc-b`
  (bob-only), `doc-pub` (public). Querying the shared term as bob returns `doc-b`
  + `doc-pub` and **never** `doc-a`; symmetric for alice.
- A **group** case: a doc visible to group `support` is seen by a member and
  hidden from a non-member.
- A **backward-compat** case: a no-ACL doc stays retrievable by an anonymous
  requester (org-public default).
- An **end-to-end runtime** case: a turn run *as bob* through
  `KnowledgeChatRuntime` + the `knowledge_search` tool never surfaces alice-only
  content in the tool result the model reads.

Plus a `can_access` unit-test matrix in `src/access_control.rs` (public,
user-match, user-no-match, group-match, group-no-match, empty-acl fully-locked,
mixed user-or-group) and `DocAcl` metadata round-trip / malformed-is-absent
tests.

## Related

- [STORAGE.md](STORAGE.md) — the `StorageAdapter` seam and the knowledge slice.
- [INGESTION.md](INGESTION.md) — where `RawDocument.acl` is stamped into `acl_v2`.
- [ONYX-TESTING-GAP-ANALYSIS.md](ONYX-TESTING-GAP-ANALYSIS.md) — G3 and the TDD plan.
