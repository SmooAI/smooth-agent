//! Document sets, curation boosting, and query-time metadata filters
//! (Onyx-gap, Phase 11) — the offline, in-memory integration tests.
//!
//! Written **test-first** (TDD): each assertion below describes a guarantee of
//! the curation layer before reaching for the implementation —
//!
//! - **set scoping**: a query scoped to set `"alpha"` returns ONLY alpha docs
//!   (never a beta-only doc); a doc in both sets shows up for either scope.
//! - **boost reorder**: a `boost=3.0` doc outranks a doc whose *raw* similarity
//!   is higher — i.e. the boost changed the order; a `boost=1.0` baseline
//!   preserves raw similarity order.
//! - **metadata filter**: `metadata_eq: {kind: prose}` returns only prose docs.
//! - **composes with ACL**: a doc in set `"alpha"` but ACL-restricted to another
//!   user is NOT returned to a user lacking access (ACL ∧ set both apply).
//! - **no-filter path unchanged**: the unconstrained reader returns the same set
//!   of docs the raw in-memory query would (boost re-ranking is a no-op at 1.0).
//!
//! All offline: the engine's `InMemoryKnowledge` backend + the curation layer in
//! our own crate. No network, no API key, no embedder needed (the in-memory KB
//! is keyword-scored).

use std::sync::Arc;

use smooth_operator::access_control::{AccessContext, DocAcl};
use smooth_operator::curation::{
    with_boost, with_document_set, CuratedKnowledgeStore, DocMeta, RetrievalFilter,
};
use smooth_operator::runtime::KnowledgeChatRuntime;
use smooth_operator::StorageAdapter;
use smooth_operator_adapter_memory::InMemoryStorageAdapter;
use smooth_operator_core::llm_provider::MockLlmClient;
use smooth_operator_core::{Document, DocumentType, InMemoryKnowledge, KnowledgeBase, LlmConfig};

/// Build a document with a fixed id (so assertions can name it) and the given
/// content + source.
fn doc(id: &str, content: &str, source: &str) -> Document {
    let mut d = Document::new(content, source, DocumentType::Documentation);
    d.id = id.to_string();
    d
}

/// Ingest a document through the store's ingest handle (the production path:
/// curation metadata + ACL are recorded as the document is stored).
fn ingest(store: &CuratedKnowledgeStore, document: Document) {
    store.ingest_handle().ingest(document).expect("ingest");
}

/// The ordered document ids a reader bound to `filter` + `access` retrieves for
/// `query`. Order is preserved (boosted-score descending) so reorder tests can
/// assert on position.
fn retrieved_ids(
    store: &CuratedKnowledgeStore,
    filter: RetrievalFilter,
    access: AccessContext,
    query: &str,
    limit: usize,
) -> Vec<String> {
    store
        .reader(filter, access)
        .query(query, limit)
        .expect("query")
        .into_iter()
        .map(|r| r.document_id)
        .collect()
}

/// SET SCOPING: a query scoped to set "alpha" returns only alpha docs (never a
/// beta-only doc); a doc that is in both sets shows up for either scope.
#[test]
fn set_scope_returns_only_alpha_docs() {
    let store = CuratedKnowledgeStore::new(Arc::new(InMemoryKnowledge::new()));

    // All three share the term "clearance" so a single query matches all three —
    // only the set membership should decide which a scoped query sees.
    ingest(
        &store,
        with_document_set(doc("a-only", "clearance alpha fact", "alpha.md"), ["alpha"]),
    );
    ingest(
        &store,
        with_document_set(doc("b-only", "clearance beta fact", "beta.md"), ["beta"]),
    );
    ingest(
        &store,
        with_document_set(
            doc("both", "clearance shared fact", "both.md"),
            ["alpha", "beta"],
        ),
    );

    // Scoped to "alpha": alpha-only + the both-doc, NEVER beta-only.
    let alpha = retrieved_ids(
        &store,
        RetrievalFilter::in_sets(["alpha"]),
        AccessContext::anonymous(),
        "clearance",
        10,
    );
    assert!(
        alpha.contains(&"a-only".to_string()),
        "alpha scope must include alpha-only; saw {alpha:?}"
    );
    assert!(
        alpha.contains(&"both".to_string()),
        "alpha scope must include the multi-set doc; saw {alpha:?}"
    );
    assert!(
        !alpha.contains(&"b-only".to_string()),
        "SET LEAK: alpha scope must NEVER include beta-only doc; saw {alpha:?}"
    );

    // Scoped to "beta": beta-only + the both-doc, NEVER alpha-only.
    let beta = retrieved_ids(
        &store,
        RetrievalFilter::in_sets(["beta"]),
        AccessContext::anonymous(),
        "clearance",
        10,
    );
    assert!(
        beta.contains(&"b-only".to_string()),
        "beta scope must include beta-only; saw {beta:?}"
    );
    assert!(
        beta.contains(&"both".to_string()),
        "the multi-set doc must appear for EITHER scope; saw {beta:?}"
    );
    assert!(
        !beta.contains(&"a-only".to_string()),
        "SET LEAK: beta scope must NEVER include alpha-only doc; saw {beta:?}"
    );
}

/// BOOST REORDER: two docs of similar similarity, one with `boost=3.0`. The
/// boosted doc ranks first **even though its raw similarity is lower** — i.e.
/// the boost changed the order. The `boost=1.0` baseline preserves raw order.
#[test]
fn boost_reorders_against_raw_similarity() {
    // The in-memory KB scores `matching_query_words / chunk_total_words`.
    //
    // For query "widget guide":
    // - "other"     = "widget guide reference"               (3 words) → 2/3 ≈ 0.667
    // - "canonical" = "widget guide reference manual extra"  (5 words) → 2/5 = 0.400
    //
    // So WITHOUT boost, "other" outranks "canonical".
    let other_content = "widget guide reference";
    let canonical_content = "widget guide reference manual extra";
    let query = "widget guide";

    // --- baseline: boost 1.0 on both → raw similarity order preserved ---
    {
        let store = CuratedKnowledgeStore::new(Arc::new(InMemoryKnowledge::new()));
        ingest(&store, doc("other", other_content, "other.md"));
        ingest(&store, doc("canonical", canonical_content, "canon.md"));
        let order = retrieved_ids(
            &store,
            RetrievalFilter::none(),
            AccessContext::anonymous(),
            query,
            10,
        );
        assert_eq!(
            order,
            vec!["other".to_string(), "canonical".to_string()],
            "baseline (no boost) must preserve raw-similarity order (higher density first)"
        );
    }

    // --- boosted: canonical gets boost 3.0 → 0.400 * 3 = 1.2 > 0.667 ---
    {
        let store = CuratedKnowledgeStore::new(Arc::new(InMemoryKnowledge::new()));
        ingest(&store, doc("other", other_content, "other.md"));
        ingest(
            &store,
            with_boost(doc("canonical", canonical_content, "canon.md"), 3.0),
        );
        let order = retrieved_ids(
            &store,
            RetrievalFilter::none(),
            AccessContext::anonymous(),
            query,
            10,
        );
        assert_eq!(
            order.first().map(String::as_str),
            Some("canonical"),
            "BOOST must promote the canonical doc above the raw-higher 'other'; saw {order:?}"
        );
        // And the order actually changed vs the baseline above.
        assert_eq!(
            order,
            vec!["canonical".to_string(), "other".to_string()],
            "boost must REORDER the two results; saw {order:?}"
        );
    }
}

/// METADATA FILTER: `metadata_eq: {kind: prose}` returns only prose docs.
#[test]
fn metadata_eq_filter_returns_only_matching_docs() {
    let store = CuratedKnowledgeStore::new(Arc::new(InMemoryKnowledge::new()));
    ingest(
        &store,
        doc("prose-1", "widget overview prose", "p1.md").with_metadata("kind", "prose"),
    );
    ingest(
        &store,
        doc("code-1", "widget overview code", "c1.rs").with_metadata("kind", "code"),
    );
    ingest(
        &store,
        doc("prose-2", "widget overview narrative", "p2.md").with_metadata("kind", "prose"),
    );

    let ids = retrieved_ids(
        &store,
        RetrievalFilter::none().with_metadata_eq("kind", "prose"),
        AccessContext::anonymous(),
        "widget overview",
        10,
    );
    assert!(
        ids.contains(&"prose-1".to_string()),
        "prose doc must pass; saw {ids:?}"
    );
    assert!(
        ids.contains(&"prose-2".to_string()),
        "prose doc must pass; saw {ids:?}"
    );
    assert!(
        !ids.contains(&"code-1".to_string()),
        "metadata filter must drop the code doc; saw {ids:?}"
    );
}

/// COMPOSES WITH ACL: a doc in set "alpha" but ACL-restricted to alice is NOT
/// returned to bob even when bob scopes the query to "alpha" — both the ACL and
/// the set filter apply (logical AND).
#[test]
fn acl_and_set_filter_both_apply() {
    let store = CuratedKnowledgeStore::new(Arc::new(InMemoryKnowledge::new()));

    // alice-only doc, in set alpha.
    let restricted = with_document_set(
        doc("alice-alpha", "clearance restricted alpha", "alice.md"),
        ["alpha"],
    );
    ingest(&store, DocAcl::for_users(["alice"]).attach_to(restricted));

    // public doc, also in set alpha (so bob has *something* legitimate to see).
    let public = with_document_set(
        doc("public-alpha", "clearance public alpha", "pub.md"),
        ["alpha"],
    );
    ingest(&store, DocAcl::public().attach_to(public));

    // Bob scopes to alpha: he sees the public alpha doc, but NEVER the
    // alice-only one — even though it IS in the requested set.
    let bob = retrieved_ids(
        &store,
        RetrievalFilter::in_sets(["alpha"]),
        AccessContext::for_user("bob"),
        "clearance",
        10,
    );
    assert!(
        bob.contains(&"public-alpha".to_string()),
        "bob should see the public alpha doc; saw {bob:?}"
    );
    assert!(
        !bob.contains(&"alice-alpha".to_string()),
        "ACL∧SET LEAK: bob must NOT see alice-only doc even within set alpha; saw {bob:?}"
    );

    // Alice, same scope, DOES see her doc (ACL grants; set matches).
    let alice = retrieved_ids(
        &store,
        RetrievalFilter::in_sets(["alpha"]),
        AccessContext::for_user("alice"),
        "clearance",
        10,
    );
    assert!(
        alice.contains(&"alice-alpha".to_string()),
        "alice should see her own alpha doc; saw {alice:?}"
    );
}

/// NO-FILTER PATH UNCHANGED: the unconstrained reader (no sets, no metadata, all
/// default boosts) returns the same documents the raw in-memory query returns.
#[test]
fn no_filter_path_matches_raw_query() {
    let inner = Arc::new(InMemoryKnowledge::new());
    let store = CuratedKnowledgeStore::new(Arc::clone(&inner) as Arc<dyn KnowledgeBase>);

    // Ingest a few docs WITHOUT any curation metadata.
    for (id, content) in [
        ("d1", "widget refund policy thirty days"),
        ("d2", "widget shipping five to seven days"),
        ("d3", "unrelated cooking recipe"),
    ] {
        ingest(&store, doc(id, content, &format!("{id}.md")));
    }

    // Raw query straight against the inner KB.
    let mut raw_ids: Vec<String> = inner
        .query("widget policy", 10)
        .unwrap()
        .into_iter()
        .map(|r| r.document_id)
        .collect();
    raw_ids.sort();
    raw_ids.dedup();

    // Unconstrained curated reader.
    let mut curated_ids = retrieved_ids(
        &store,
        RetrievalFilter::none(),
        AccessContext::anonymous(),
        "widget policy",
        10,
    );
    curated_ids.sort();
    curated_ids.dedup();

    assert_eq!(
        curated_ids, raw_ids,
        "unconstrained curated reader must surface the same docs as the raw query"
    );
}

/// A document with no recorded curation metadata belongs to no named set, so a
/// set-scoped query never surfaces it (only docs explicitly tagged into the set
/// appear) — the complement of the no-filter path.
#[test]
fn untagged_doc_is_in_no_set() {
    let store = CuratedKnowledgeStore::new(Arc::new(InMemoryKnowledge::new()));
    ingest(&store, doc("untagged", "widget clearance fact", "u.md"));
    ingest(
        &store,
        with_document_set(doc("tagged", "widget clearance fact", "t.md"), ["alpha"]),
    );

    let scoped = retrieved_ids(
        &store,
        RetrievalFilter::in_sets(["alpha"]),
        AccessContext::anonymous(),
        "widget clearance",
        10,
    );
    assert!(
        scoped.contains(&"tagged".to_string()),
        "tagged doc in set alpha must appear; saw {scoped:?}"
    );
    assert!(
        !scoped.contains(&"untagged".to_string()),
        "an untagged doc belongs to no set, so a set-scoped query must skip it; saw {scoped:?}"
    );
}

/// END-TO-END through `KnowledgeChatRuntime` + the `knowledge_search` tool: a
/// turn run with the runtime scoped to set "alpha" must surface only alpha-set
/// content in the tool result the model reads — the curation guarantee at the
/// runtime seam, not just the store seam. Driven by a `MockLlmClient` (no API
/// key, no network).
#[tokio::test]
async fn runtime_curation_scopes_knowledge_search_to_set() {
    // The runtime reads through `storage.knowledge()`; wrap THAT exact handle in
    // a CuratedKnowledgeStore so the side table and the inner store agree.
    let storage = Arc::new(InMemoryStorageAdapter::new());
    let store = CuratedKnowledgeStore::new(storage.knowledge());
    let h = store.ingest_handle();

    // An alpha-set doc and a beta-set doc, both matching "frobnicator".
    h.ingest(with_document_set(
        doc(
            "alpha-doc",
            "The frobnicator alpha subsystem uses a 42-slot ring buffer.",
            "alpha/frob.md",
        ),
        ["alpha"],
    ))
    .expect("ingest alpha doc");
    h.ingest(with_document_set(
        doc(
            "beta-doc",
            "The frobnicator beta subsystem uses an 88-slot ring buffer.",
            "beta/frob.md",
        ),
        ["beta"],
    ))
    .expect("ingest beta doc");

    // Script the model to issue a knowledge_search for "frobnicator".
    let mock = MockLlmClient::new();
    mock.push_tool_call(
        "call_1",
        "knowledge_search",
        serde_json::json!({ "query": "frobnicator ring buffer" }),
    )
    .push_text("Here is what I found.");

    let llm = LlmConfig::openrouter("not-a-real-key").with_model("openai/gpt-4o");
    let runtime = KnowledgeChatRuntime::new(storage.clone(), llm)
        .with_llm_provider(Arc::new(mock.clone()))
        // Scope the whole turn to set "alpha".
        .with_curation(
            store,
            AccessContext::anonymous(),
            RetrievalFilter::in_sets(["alpha"]),
        );

    let outcome = runtime
        .run_turn("conv-curation", "Tell me about the frobnicator")
        .await
        .expect("run_turn");

    let tool_result = outcome
        .tool_result("knowledge_search")
        .expect("knowledge_search ran");

    assert!(
        tool_result.contains("42-slot"),
        "alpha-set content should surface; tool result: {tool_result}"
    );
    assert!(
        !tool_result.contains("88-slot"),
        "SET LEAK: beta-set content reached the model under an alpha scope; tool result: {tool_result}"
    );
}

/// Sanity: `DocMeta` parsed from a stored document round-trips the set + boost
/// the builders stamped (the ingest-time record the reader consults).
#[test]
fn docmeta_round_trips_set_and_boost() {
    let d = with_boost(
        with_document_set(doc("x", "content", "x.md"), ["alpha", "beta"]),
        2.5,
    );
    let meta = DocMeta::from_document(&d);
    assert_eq!(
        meta.document_sets,
        vec!["alpha".to_string(), "beta".to_string()]
    );
    assert!((meta.boost - 2.5).abs() < f32::EPSILON);
    assert!(meta.in_set("alpha") && meta.in_set("beta") && !meta.in_set("gamma"));
}
