# Document-level access control

Mature knowledge platforms sync per-connector permissions and filter retrieval by user entitlement;
before this, smooth-operator filtered knowledge by `organizationId` only.
This is the within-org **document-level** layer (feature gap **G3**, the
highest-severity gap in [FEATURE-GAP-ANALYSIS.md](FEATURE-GAP-ANALYSIS.md)):
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

This wrapper is the **in-memory** enforcement path. Its ACL side table is
process-local, so it cannot carry a document's ACL from the ingestion process to
a separate serving process. For the durable backends the ACL is therefore
**persisted with the document** and enforced from storage at read (see
[Durable persistence](#durable-persistence-postgres--dynamodb) below) — so the
guarantee survives the ingest→serve boundary, not just a single process.

## The `StorageAdapter` ACL seam — `knowledge_for_access`

Every backend exposes two knowledge handles through the `StorageAdapter` trait
(`smooth-operator/src/adapter.rs`):

- **`knowledge()`** — org isolation only. Used by ingest / admin / seeding. It
  does **not** enforce within-org ACLs.
- **`knowledge_for_access(&AccessContext)`** — an **ACL-enforcing** handle bound
  to the requester. Its `query` returns only documents the requester is entitled
  to read. **This is the handle the chat retrieval path MUST use.**

Per backend:

| Backend   | `knowledge_for_access` enforcement |
| --------- | ---------------------------------- |
| In-memory | Wraps the shared `AclKnowledgeStore` reader (side table populated at ingest). |
| Postgres  | A `PgKnowledgeBase` clone bound to the `AccessContext`; filters in SQL against the stored `acl` column (a restricted row is never even fetched). |
| DynamoDB  | A `DynamoKnowledgeBase` clone bound to the `AccessContext`; post-filters the brute-force scan against each item's stored `acl` attribute. |

The default trait impl wraps `knowledge()` in an `AclKnowledgeStore` reader with
an empty side table (every doc treated as org-public — the raw `knowledge()`
behavior, not a regression). The three real backends override it to enforce
durably.

## Enforcement on the live chat path (server + lambda)

> This closed the **#1 adversarial-review security finding**: the ACL layer was
> dead on the live chat path, so a private GitHub repo was retrievable by **any**
> chat user. The runner queried `storage.knowledge()` **raw** — no
> `AccessContext`, no ACL reader — for both the auto-injected context and the
> `knowledge_search` tool.

The streaming chat runner (`smooth-operator-server/src/runner.rs`,
`run_streaming_turn`) — used by **both** the reference WS server
(`handler.rs`) and the production AWS Lambda (`smooth-operator-lambda/src/dispatch.rs`) —
now takes an `AccessContext` on its `TurnRequest` and builds **one**
`storage.knowledge_for_access(&access)` handle that feeds **both** retrieval
surfaces:

1. the engine's auto-injected `[Relevant knowledge]` context, and
2. the agent's `knowledge_search` tool.

A restricted document is dropped before it can reach the model **or** a citation.

### `/ws` authentication → `AccessContext`

- **Reference server**: the bearer JWT rides on the `?token=` query param of the
  `/ws` upgrade (browsers can't set custom headers on a WebSocket handshake). It
  is verified once at connect via the configured `AuthVerifier`, mapped to the
  `Principal`'s `AccessContext`, and threaded into every turn on that connection.
- **Lambda**: API Gateway WebSocket has no persistent socket, so the token rides
  on the `send_message` frame (a `token` field), verified per frame.

**Fail closed for ACL'd content.** When no token is presented, the verifier is
unconfigured/disabled (dev/no-auth), or the token fails to verify, the connection
runs as `AccessContext::anonymous()` — which sees **only org-public** knowledge,
**not** every document. Verification failures are logged (never the token) and
degrade to anonymous rather than dropping the connection, so the dev/no-auth case
still serves org-public knowledge.

### Groups come from the JWT

`Principal::access_context()` now populates **both** the user id and the
principal's **groups**, parsed from a `groups` claim on the JWT (`auth.rs`,
`Claims.groups`). This is what lets an authenticated user match a
`github:owner/repo` document ACL — a private-repo doc scoped to that group is
readable only by a principal carrying it.

## Durable persistence (Postgres + DynamoDB)

The in-memory ACL side table dies with its process. The durable backends persist
the `DocAcl` **with the document** so the ACL survives ingest(process)→serve(process):

- **Postgres** — a `knowledge_vectors.acl JSONB` column, written at ingest from
  the `acl_v2` metadata. `query_async` filters **in SQL**: a row is visible when
  `acl IS NULL` (org-public) OR `acl->>'public'` is true OR the requester's user
  id is in `acl->'users'` (jsonb `?`) OR any requester group is in `acl->'groups'`
  (jsonb `?|`). The column is added idempotently (`ADD COLUMN IF NOT EXISTS`) so
  an in-place upgrade picks it up.
- **DynamoDB** — an `acl` string attribute on each knowledge item; the
  brute-force scan parses it back and post-filters via `can_access`.

"No ACL recorded ⇒ org-public" holds identically across all three backends.

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

### Chat-path + persistence + cross-org tests (the live-path hardening)

- **The headline chat-path leak test** — `smooth-operator-server/tests/acl_chat_leak.rs`
  (written first, failed before the runner threaded an `AccessContext`). It runs
  the **real** `run_streaming_turn` offline (a `MockLlmClient` scripts the
  streaming `knowledge_search` call) over an in-memory store seeded with an
  org-public doc and a private-repo doc scoped to group `github:acme/secret`,
  and asserts: a user **without** the group (and an anonymous connection) never
  see the private doc in the tool result the model reads **or** in any citation;
  a user **with** the group does.
- **Postgres persistence** — `adapters/postgres/tests/acl_persistence.rs`
  (testcontainers): ingest an ACL'd doc through one adapter, then query through a
  **fresh** adapter (a different process, in production) → the ACL is enforced
  from the `acl` column, proving it survives the ingest→serve boundary.
- **Groups-from-JWT** — `src/auth.rs` unit tests: a token's `groups` claim
  surfaces on the `Principal` and its `AccessContext`, and a tokenless principal
  cannot match a group-scoped doc.
- **Cross-org admin scoping** — `smooth-operator-server/tests/admin_api.rs`:
  org A's indexing runs + document sets are invisible to an org-B caller, and
  two orgs with a same-named connector don't collide (see [ADMIN-API.md](ADMIN-API.md)).

## Related

- [STORAGE.md](STORAGE.md) — the `StorageAdapter` seam and the knowledge slice.
- [INGESTION.md](INGESTION.md) — where `RawDocument.acl` is stamped into `acl_v2`.
- [FEATURE-GAP-ANALYSIS.md](FEATURE-GAP-ANALYSIS.md) — G3 and the TDD plan.
