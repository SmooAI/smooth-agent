//! Headline acceptance test for the ingestion pipeline (feature gap G1).
//!
//! TDD contract (written before the implementation): build a real in-memory
//! `StorageAdapter` + a `DeterministicEmbedder` + a `MockConnector` carrying a
//! couple of distinctive documents, run `ingest(...)`, and assert the full
//! chunk → embed → store → retrieve round-trip plus idempotency:
//!
//! (a) the connector's documents are chunked and landed in the knowledge slice,
//! (b) a retrieval query for a distinctive term returns the seeded chunk first,
//! (c) re-running `ingest` is idempotent — no duplicate chunks accumulate.
//!
//! No network, no credentials: the connector is a fixture and the embedder is
//! deterministic, so this runs on every PR.

use std::sync::Arc;

use smooth_operator::adapter::StorageAdapter;
use smooth_operator_adapter_memory::InMemoryStorageAdapter;

use smooth_operator_ingestion::{
    ingest, Chunker, DeterministicEmbedder, IngestLedger, IngestOptions, MockConnector, RawDocument,
};

/// Two distinctive documents whose salient terms ("zorblax", "quibbleton") do
/// not collide with each other or with ordinary English, so retrieval scoring
/// is unambiguous.
fn fixture_docs() -> Vec<RawDocument> {
    vec![
        RawDocument::new(
            "doc-zorblax",
            "mock",
            "The zorblax is a rare crystalline organism. \
             A zorblax glows faintly under moonlight and feeds on static electricity. \
             Zorblax colonies are found only in the Quibbleton highlands.",
        )
        .with_title("Zorblax Facts")
        .with_metadata("category", "fauna"),
        RawDocument::new(
            "doc-flooble",
            "mock",
            "Flooble engineering is the practice of bending narrow beams. \
             A flooble joint distributes load across three anchor points.",
        )
        .with_title("Flooble Engineering"),
    ]
}

#[tokio::test]
async fn ingest_chunks_embeds_stores_and_retrieves_then_is_idempotent() {
    let storage: Arc<dyn StorageAdapter> = Arc::new(InMemoryStorageAdapter::new());
    let connector = MockConnector::new(fixture_docs());
    let chunker = Chunker::default();
    let embedder = DeterministicEmbedder::new();
    // The ledger is the durable dedup state. It persists across ingest runs
    // (the engine's KnowledgeBase has no list/delete, so idempotency is the
    // ingestion layer's responsibility). A production backend would back this
    // with the same DB; in-memory here.
    let ledger = IngestLedger::default();

    // ---- first ingest -----------------------------------------------------
    let report = ingest(
        &connector,
        &chunker,
        &embedder,
        storage.knowledge(),
        IngestOptions::for_org("org-acme").with_ledger(ledger.clone()),
    )
    .await
    .expect("first ingest succeeds");

    // (a) Both documents were pulled and produced at least one chunk each.
    assert_eq!(report.documents_pulled, 2, "pulled both fixture docs");
    assert!(
        report.chunks_stored >= 2,
        "expected at least one chunk per doc, got {}",
        report.chunks_stored
    );

    // (b) A distinctive query returns the matching doc's chunk first.
    let kb = storage.knowledge();
    let hits = kb.query("zorblax", 5).expect("query knowledge base");
    assert!(!hits.is_empty(), "zorblax query returned nothing");
    assert!(
        hits[0].chunk.to_lowercase().contains("zorblax"),
        "top hit should be the zorblax chunk, got: {}",
        hits[0].chunk
    );
    // The unrelated flooble doc must not be the top hit for a zorblax query.
    assert!(
        !hits[0].chunk.to_lowercase().contains("flooble"),
        "flooble chunk leaked to the top of a zorblax query"
    );

    // Snapshot how many chunks exist after the first run (count distinct
    // chunks the store will return across a broad query).
    let broad_first = kb.query("zorblax flooble", 100).expect("broad query");
    let count_first = broad_first.len();
    assert!(
        count_first >= 2,
        "expected >=2 stored chunks, got {count_first}"
    );

    // ---- second ingest (idempotency) -------------------------------------
    let report2 = ingest(
        &connector,
        &chunker,
        &embedder,
        storage.knowledge(),
        IngestOptions::for_org("org-acme").with_ledger(ledger.clone()),
    )
    .await
    .expect("second ingest succeeds");

    // Same documents, same content → nothing new should be stored.
    assert_eq!(
        report2.chunks_stored, 0,
        "re-ingesting identical content must store zero new chunks (idempotent)"
    );
    assert_eq!(
        report2.documents_skipped, 2,
        "both unchanged documents should be skipped on re-ingest"
    );

    // (c) The store did not grow.
    let broad_second = kb.query("zorblax flooble", 100).expect("broad query");
    assert_eq!(
        broad_second.len(),
        count_first,
        "re-ingest duplicated chunks: {} before, {} after",
        count_first,
        broad_second.len()
    );
}
