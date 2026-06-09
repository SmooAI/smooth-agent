//! Document-level access control (feature gap G3) — the cross-user **leak** test.
//!
//! This test is the highest-severity guarantee in the gap analysis: within a
//! single organization, a retrieval as user B must **never** surface a document
//! that is restricted to user A. Org isolation (via `organizationId`) already
//! exists; this adds the within-org user/group entitlement layer on top.
//!
//! It is written **test-first** (TDD): before the ACL-aware retrieval path
//! existed, a query for a term shared across docs returned every matching doc
//! regardless of who asked — the `never_sees_a_only` assertion below failed.
//! The fix threads an [`AccessContext`] into retrieval and drops results the
//! requester is not entitled to.

use std::sync::Arc;

use smooth_operator::access_control::{AccessContext, AclKnowledgeStore, DocAcl};
use smooth_operator::runtime::KnowledgeChatRuntime;
use smooth_operator::StorageAdapter;
use smooth_operator_adapter_memory::InMemoryStorageAdapter;
use smooth_operator_core::llm_provider::MockLlmClient;
use smooth_operator_core::{Document, DocumentType, LlmConfig};

/// Seed three docs into a fresh ACL-aware store, each sharing the term
/// `"clearance"` so a single query matches all three — only the ACL should
/// decide which a given requester sees.
///
/// - `doc-a`   → users:[alice]      (alice-only)
/// - `doc-b`   → users:[bob]        (bob-only)
/// - `doc-pub` → public:true        (everyone in the org)
fn seeded_store() -> AclKnowledgeStore {
    // Wrap the engine's in-memory KB; ACL enforcement is entirely in our layer.
    let store = AclKnowledgeStore::new(Arc::new(smooth_operator_core::InMemoryKnowledge::new()));

    ingest(
        &store,
        "doc-a",
        "alice/secret.md",
        "Project clearance alpha is restricted to alice.",
        DocAcl::for_users(["alice"]),
    );
    ingest(
        &store,
        "doc-b",
        "bob/secret.md",
        "Project clearance bravo is restricted to bob.",
        DocAcl::for_users(["bob"]),
    );
    ingest(
        &store,
        "doc-pub",
        "shared/handbook.md",
        "General clearance procedures are public to the whole org.",
        DocAcl::public(),
    );
    store
}

/// Ingest one document with an attached [`DocAcl`] through the store's ingest
/// handle (the same path the ingestion pipeline uses: ACL serialized into the
/// document metadata).
fn ingest(store: &AclKnowledgeStore, id: &str, source: &str, content: &str, acl: DocAcl) {
    let mut doc = Document::new(content, source, DocumentType::Documentation);
    doc.id = id.to_string();
    let doc = acl.attach_to(doc);
    store.ingest_handle().ingest(doc).expect("ingest");
}

/// Collect the document ids a reader bound to `ctx` can retrieve for `query`.
fn visible_ids(store: &AclKnowledgeStore, ctx: AccessContext, query: &str) -> Vec<String> {
    let reader = store.reader(ctx);
    let mut ids: Vec<String> = reader
        .query(query, 10)
        .expect("query")
        .into_iter()
        .map(|r| r.document_id)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// THE LEAK TEST: user B, querying the shared term, sees doc-B and the public
/// doc — but **never** doc-A (alice-only). Symmetric for user A.
#[tokio::test]
async fn cross_user_query_never_leaks_other_users_doc() {
    let store = seeded_store();

    // --- as bob ---
    let bob = AccessContext::for_user("bob");
    let bob_sees = visible_ids(&store, bob, "clearance");
    assert!(
        bob_sees.contains(&"doc-b".to_string()),
        "bob should see his own doc; saw {bob_sees:?}"
    );
    assert!(
        bob_sees.contains(&"doc-pub".to_string()),
        "bob should see the public doc; saw {bob_sees:?}"
    );
    assert!(
        !bob_sees.contains(&"doc-a".to_string()),
        "LEAK: bob must NEVER see alice-only doc-a; saw {bob_sees:?}"
    );

    // --- as alice (symmetric) ---
    let alice = AccessContext::for_user("alice");
    let alice_sees = visible_ids(&store, alice, "clearance");
    assert!(
        alice_sees.contains(&"doc-a".to_string()),
        "alice should see her own doc; saw {alice_sees:?}"
    );
    assert!(
        alice_sees.contains(&"doc-pub".to_string()),
        "alice should see the public doc; saw {alice_sees:?}"
    );
    assert!(
        !alice_sees.contains(&"doc-b".to_string()),
        "LEAK: alice must NEVER see bob-only doc-b; saw {alice_sees:?}"
    );
}

/// Group-based entitlement: a doc visible to group "support" is retrievable by
/// a member of that group and hidden from a non-member.
#[tokio::test]
async fn group_membership_gates_retrieval() {
    let store = AclKnowledgeStore::new(Arc::new(smooth_operator_core::InMemoryKnowledge::new()));
    ingest(
        &store,
        "doc-support",
        "internal/runbook.md",
        "Escalation runbook tango for the support team.",
        DocAcl::for_groups(["support"]),
    );

    // In-group member sees it.
    let member = AccessContext::new(Some("carol".to_string()), vec!["support".to_string()]);
    let member_sees = visible_ids(&store, member, "tango");
    assert!(
        member_sees.contains(&"doc-support".to_string()),
        "support-group member should see the support doc; saw {member_sees:?}"
    );

    // Out-of-group user does not.
    let outsider = AccessContext::new(Some("dave".to_string()), vec!["billing".to_string()]);
    let outsider_sees = visible_ids(&store, outsider, "tango");
    assert!(
        !outsider_sees.contains(&"doc-support".to_string()),
        "LEAK: non-member must NOT see the support-group doc; saw {outsider_sees:?}"
    );
}

/// Backward-compat: a document ingested with **no** ACL (the legacy path —
/// existing seeded knowledge) stays retrievable by everyone. No-acl ⇒
/// org-public is the documented default.
#[tokio::test]
async fn no_acl_document_is_org_public_for_backward_compat() {
    let store = AclKnowledgeStore::new(Arc::new(smooth_operator_core::InMemoryKnowledge::new()));
    // Plain document, no ACL metadata attached.
    let mut doc = Document::new(
        "Legacy seeded fact about widget warranties.",
        "legacy/widgets.md",
        DocumentType::Documentation,
    );
    doc.id = "doc-legacy".to_string();
    store.ingest_handle().ingest(doc).expect("ingest legacy");

    // An anonymous requester (no user, no groups) still retrieves it.
    let anon = AccessContext::anonymous();
    let seen = visible_ids(&store, anon, "warranties");
    assert!(
        seen.contains(&"doc-legacy".to_string()),
        "no-acl doc must remain retrievable (org-public default); saw {seen:?}"
    );
}

/// End-to-end through `KnowledgeChatRuntime` + the `knowledge_search` tool: with
/// access control wired into the runtime, a turn run *as bob* must never surface
/// alice-only content in the tool result the model reads — the leak guarantee at
/// the runtime seam, not just the store seam. Driven by a MockLlmClient so it
/// runs with no API key and no network.
#[tokio::test]
async fn runtime_knowledge_search_is_access_controlled() {
    // The runtime reads through `storage.knowledge()`; wrap THAT exact handle in
    // an AclKnowledgeStore so the ACL side table and the inner store agree.
    let storage = Arc::new(InMemoryStorageAdapter::new());
    let acl_store = AclKnowledgeStore::new(storage.knowledge());

    // Ingest alice-only + public docs through the ACL store's ingest handle so
    // the inner KB (== storage.knowledge()) holds the chunks and the ACL table
    // records who may read each.
    let ingest = acl_store.ingest_handle();
    let mut a = Document::new(
        "The alpha launch codes are restricted to alice only.",
        "alice/codes.md",
        DocumentType::Documentation,
    );
    a.id = "doc-a".to_string();
    ingest
        .ingest(DocAcl::for_users(["alice"]).attach_to(a))
        .expect("ingest alice doc");
    let mut p = Document::new(
        "The alpha office hours are public to the whole org.",
        "shared/hours.md",
        DocumentType::Documentation,
    );
    p.id = "doc-pub".to_string();
    ingest
        .ingest(DocAcl::public().attach_to(p))
        .expect("ingest public doc");

    // Script the model to issue a knowledge_search for the shared term "alpha".
    let mock = MockLlmClient::new();
    mock.push_tool_call(
        "call_1",
        "knowledge_search",
        serde_json::json!({ "query": "alpha" }),
    )
    .push_text("Here is what I found.");

    let llm = LlmConfig::openrouter("not-a-real-key").with_model("openai/gpt-4o");
    let runtime = KnowledgeChatRuntime::new(storage.clone(), llm)
        .with_llm_provider(Arc::new(mock.clone()))
        // Run the turn AS BOB — entitled to neither alice's doc.
        .with_access_control(acl_store, AccessContext::for_user("bob"));

    let outcome = runtime
        .run_turn("conv-acl", "Tell me about alpha")
        .await
        .expect("run_turn");

    let tool_result = outcome
        .tool_result("knowledge_search")
        .expect("knowledge_search ran");

    // bob sees the public doc...
    assert!(
        tool_result.contains("office hours"),
        "bob should see the public alpha doc; tool result: {tool_result}"
    );
    // ...but the alice-only doc must NEVER appear in what the model reads.
    assert!(
        !tool_result.contains("launch codes"),
        "LEAK: alice-only content reached the model as bob; tool result: {tool_result}"
    );
    assert!(
        !tool_result.contains("alice/codes.md"),
        "LEAK: alice-only source reached the model as bob; tool result: {tool_result}"
    );
}
