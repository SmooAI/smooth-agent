# Knowledge ingestion & connectors

smooth-operator-agent's knowledge base used to be seeded by hand: you called
`KnowledgeBase::ingest(doc)` yourself. The **ingestion crate**
(`rust/ingestion`, package `smooai-smooth-operator-agent-ingestion`) closes that
gap — it pulls documents from a *source*, chunks them, embeds them, and stores
them in the `StorageAdapter` knowledge slice so they're retrievable. This is the
analog of [Onyx's connector framework](ONYX-TESTING-GAP-ANALYSIS.md) and closes
gaps **G1** (ingestion + connectors), **G2** (chunking pipeline), and **G9** (the
connector mock + `unit`-vs-`external` test split).

## The pipeline

```text
Connector::pull ─▶ Chunker::chunk ─▶ Embedder::embed ─▶ KnowledgeBase::ingest
   RawDocument        Vec<Chunk>        Vec<Vec<f32>>      (StorageAdapter
                                                            knowledge slice)
```

One call drives the whole thing:

```rust
use std::sync::Arc;
use smooth_operator_agent_core::adapter::StorageAdapter;
use smooth_operator_agent_ingestion::{
    ingest, Chunker, DeterministicEmbedder, FileConnector, IngestLedger, IngestOptions,
};

let storage: Arc<dyn StorageAdapter> = /* in-memory / Postgres / DynamoDB */;
let connector = FileConnector::new("./docs");
let ledger = IngestLedger::default(); // persist & reuse for idempotent re-runs

let report = ingest(
    &connector,
    &Chunker::default(),
    &DeterministicEmbedder::new(),
    storage.knowledge(),
    IngestOptions::for_org("org-acme").with_ledger(ledger),
)
.await?;

println!("pulled {}, stored {} chunks, skipped {} docs",
    report.documents_pulled, report.chunks_stored, report.documents_skipped);
```

`ingest(connector, chunker, embedder, knowledge, options) -> IngestReport`:

1. **pull** the connector's `RawDocument`s (optionally only those changed since a
   `since` watermark),
2. **chunk** each document (paragraph/size split with overlap; see below),
3. **embed** each new chunk as a batch (dimension validated),
4. **store** each chunk as a one-chunk `Document` in the knowledge slice,
5. dedupe on `(org, document id, chunk content hash)` via the `IngestLedger` so a
   re-ingest over unchanged sources stores **nothing new** (idempotent).

`IngestReport` carries `documents_pulled`, `documents_skipped`, `chunks_stored`,
and `embedding_dim`.

### Why the pipeline embeds even though `KnowledgeBase::ingest` re-embeds

The engine's `KnowledgeBase` trait takes a whole `Document` and owns its own
embedding (the Postgres `PgKnowledgeBase` embeds inside `ingest`; the in-memory
one is keyword-scored). The pipeline still runs the `Embedder` per chunk so the
embed step is a **first-class, tested stage** (dimension validated, batch path
exercised) and so a backend that accepts a precomputed vector can be wired
without changing pipeline code. The computed vectors are surfaced on
`IngestReport`, not discarded.

### Idempotency

The engine's `KnowledgeBase` has no `list`/`delete`, so idempotency lives in the
ingestion layer. The `IngestLedger` is a cheap-to-clone `Arc` handle holding the
set of `(org, doc id, content hash)` keys already stored. **Share one ledger
across runs** to get idempotency; a fresh ledger (the default) re-stores every
run. A production deployment persists the ledger alongside the knowledge store.
Changed content → new hash → re-ingested; unchanged content → skipped.

## The `Connector` trait

```rust
#[async_trait]
pub trait Connector: Send + Sync {
    fn name(&self) -> &str;
    async fn pull(&self, since: Option<Timestamp>) -> Result<Vec<RawDocument>>;
}
```

`RawDocument` is the normalized payload every connector returns:

| field      | meaning                                                              |
| ---------- | ------------------------------------------------------------------- |
| `id`       | connector-stable identity (file path, URL, record id) — dedup key   |
| `source`   | origin label (`"file"`, `"web"`, …)                                 |
| `title`    | optional human title (folded into chunk metadata)                   |
| `content`  | textual content (HTML already stripped for the web case)            |
| `metadata` | arbitrary source metadata, propagated onto every chunk              |
| `acl`      | optional access-control labels — stamped into the doc's `acl_v2` `DocAcl` (as group entitlements) and enforced at read; see [ACCESS-CONTROL.md](ACCESS-CONTROL.md) |

Return a **stable `id`** for the same logical document so re-ingests can skip
unchanged content.

## Built-in connectors

| Connector        | Source                              | Tests                                  |
| ---------------- | ----------------------------------- | -------------------------------------- |
| `MockConnector`  | a fixed `Vec<RawDocument>`          | the contract test fixture (`unit`)     |
| `FileConnector`  | a `.txt`/`.md` file or directory    | `unit` (temp dirs, no network)         |
| `WebConnector`   | a public URL → HTML→text            | `unit` (offline strip/guard) + gated live |

- **`FileConnector::new(path)`** — a file yields one document; a directory is
  walked recursively (`.non_recursive()` to stay top-level) yielding one document
  per `.txt`/`.md`/`.markdown`/`.mdx`/`.text` file. `id` = the file path (stable),
  `title` = the file stem.
- **`WebConnector::new(url)`** — fetches one public URL and returns one document.
  It **reuses the engine's `fetch_url` internals** (`assert_url_is_public` SSRF
  guard + `html_to_text` stripper) so there is exactly one copy of the
  strip/guard logic, shared with the agent's `fetch_url` tool — no drift. The
  SSRF guard runs *before* any request: loopback / private / link-local /
  metadata / non-http(s) URLs are rejected.

### Stubbed (follow-up)

A **`github`** connector (repo files / issues / wikis via the GitHub API) and the
broader SaaS set Onyx covers (confluence, jira, notion, slack, salesforce, …) are
intentionally not in this batch — each is a new `Connector` impl following the
authoring recipe below. Don't over-scope; add them one at a time with the same
test split.

## The chunker (G2)

```rust
Chunker::new(max_chars, overlap_chars)   // or Chunker::default() (500 / 64)
```

Strategy:

1. split content into paragraphs on blank lines (`\n\n`),
2. greedily pack paragraphs into a chunk up to `max_chars`,
3. a single paragraph larger than the cap is hard-split on word boundaries,
4. successive chunks overlap by `overlap_chars` of trailing whole words so a
   fact spanning a boundary stays retrievable.

`overlap_chars` is clamped below `max_chars` so chunking always makes forward
progress. Each `Chunk` gets a **stable id** `"{doc_id}#{index}"` and inherits the
source document's title, metadata (`title`, `source`, plus any custom keys), and
`acl`. The `acl` labels become a `DocAcl` (under the `acl_v2` metadata key) that
ACL-filtered retrieval enforces at read — see [ACCESS-CONTROL.md](ACCESS-CONTROL.md).

## The embedder seam

```rust
#[async_trait]
pub trait Embedder: Send + Sync {
    fn dim(&self) -> usize;
    async fn embed(&self, texts: &[String], input_type: InputType) -> Result<Vec<Vec<f32>>>;
}
```

`DeterministicEmbedder` is the default: a network-free, FNV-1a token-hashing,
L2-normalized pseudo-embedder (1024-d) — same text → same vector, shared-token
texts land closer. This is the **same `Embedder` seam** the Postgres adapter
defines (`adapters/postgres/src/embedder.rs`), and the two `DeterministicEmbedder`
implementations are byte-identical (same hashing, normalization, default
dimension → same vectors). It is kept as a minimal local copy rather than a
shared import so the ingestion crate doesn't drag in `deadpool-postgres` /
`tokio-postgres`; if the seam ever moves to a shared crate, both sites adopt it.
A provider-backed embedder (e.g. the gateway embedder) just implements the same
trait.

## Authoring a custom connector

1. Define a struct holding whatever it needs (a base URL, an API client, creds).
2. `impl Connector`: `name()` returns a short label; `pull(since)` returns
   `Vec<RawDocument>` (honor `since` if the source supports incremental sync,
   otherwise ignore it — the `(id, hash)` dedup keeps full re-pulls cheap).
3. Give each document a **stable `id`** so re-ingests dedupe.
4. Tests follow the **G9 split**:
   - a **`unit`** test against fixture data (no creds, no network) — runs every
     PR. For HTTP connectors, factor the parse/transform into a pure function and
     test it offline (see `WebConnector::body_to_doc`).
   - an **external/live** test that touches the real source, marked `#[ignore]`
     and gated on `SMOOTH_AGENT_E2E=1`, so credential-free CI skips it and nightly
     runs it (mirrors `WebConnector::live_fetch_example`).

## Tests & the unit-vs-external split (G9)

| Tier        | What                                                  | When           |
| ----------- | ---------------------------------------------------- | -------------- |
| `unit`      | chunker, embedder, file connector, web strip/guard, `tests/ingestion_contract.rs` (chunk→embed→store→retrieve + idempotency) | every PR, no creds |
| `external`  | `WebConnector::live_fetch_example` (real fetch)      | gated on `SMOOTH_AGENT_E2E=1`, nightly |

The headline acceptance is `rust/ingestion/tests/ingestion_contract.rs`: it wires
an in-memory `StorageAdapter` + `DeterministicEmbedder` + `MockConnector`, runs
`ingest`, and asserts (a) documents were chunked and stored, (b) a distinctive
query returns the seeded chunk first, and (c) re-running `ingest` stores zero new
chunks (idempotent).

Run them:

```bash
cd rust
cargo test -p smooai-smooth-operator-agent-ingestion          # unit tier
SMOOTH_AGENT_E2E=1 cargo test -p smooai-smooth-operator-agent-ingestion \
  -- --ignored                                                # + live web fetch
```

## Source files

- `rust/ingestion/src/connector.rs` — `Connector` trait, `RawDocument`, `MockConnector`
- `rust/ingestion/src/chunker.rs` — `Chunker`, `Chunk` (G2)
- `rust/ingestion/src/embedder.rs` — `Embedder`, `DeterministicEmbedder`
- `rust/ingestion/src/pipeline.rs` — `ingest`, `IngestOptions`, `IngestLedger`, `IngestReport`
- `rust/ingestion/src/connectors/file.rs` — `FileConnector`
- `rust/ingestion/src/connectors/web.rs` — `WebConnector`
- `rust/ingestion/tests/ingestion_contract.rs` — headline acceptance test

## Related

- [STORAGE.md](STORAGE.md) — the `StorageAdapter` seam and its knowledge slice
- [TOOLS.md](TOOLS.md) — `fetch_url` (whose SSRF guard + HTML stripper the web connector reuses) and `knowledge_search` (retrieval over what ingestion stores)
- [ONYX-TESTING-GAP-ANALYSIS.md](ONYX-TESTING-GAP-ANALYSIS.md) — G1/G2/G9 this closes
