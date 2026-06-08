//! Offline smoke test for the dev-support example — **no network, no real
//! GitHub, no API key**.
//!
//! It mimics a tiny repo with a `MockConnector` (a README with a distinctive
//! fact + a code file), ingests it through the example's real pipeline, then runs
//! one turn through [`DevSupportRuntime`] with a scripted
//! [`MockLlmClient`](smooth_operator_core::llm_provider::MockLlmClient) that
//! decides to call `knowledge_search`. The test asserts the agent retrieved the
//! repo fact and the turn completed grounded in it — exercising config → ingest →
//! runtime exactly as `dev-support chat` would, minus the binary and the live LLM.

use std::sync::Arc;

use dev_support::config::DevSupportConfig;
use dev_support::ingest::ingest_into_memory;
use dev_support::runtime::{tool_github_auth, DevSupportRuntime};

use smooth_operator::StorageAdapter;
use smooth_operator_core::llm_provider::MockLlmClient;
use smooth_operator_core::LlmConfig;
use smooth_operator_ingestion::{MockConnector, RawDocument};

/// A distinctive fact that a generic, ungrounded answer could not invent.
const DISTINCTIVE_FACT: &str = "the Frobnicator uses a 42-slot ring buffer";

/// A fixture "repo": a README mentioning the distinctive fact + a code file.
fn fixture_connector() -> MockConnector {
    MockConnector::new(vec![
        RawDocument::new(
            "acme/widget@main#README.md",
            "https://github.com/acme/widget/blob/main/README.md",
            format!(
                "# Widget\n\nThe Widget service is built around the Frobnicator subsystem. \
                 Internally {DISTINCTIVE_FACT} to batch incoming events before flushing them."
            ),
        )
        .with_title("README.md")
        .with_metadata("kind", "prose")
        .with_metadata("repo", "acme/widget"),
        RawDocument::new(
            "acme/widget@main#src/frob.rs",
            "https://github.com/acme/widget/blob/main/src/frob.rs",
            "pub struct Frobnicator { ring: [Event; 42] }\n\nimpl Frobnicator { pub fn flush(&self) {} }",
        )
        .with_title("src/frob.rs")
        .with_metadata("kind", "code")
        .with_metadata("lang", "rust"),
    ])
}

/// The example config used by the smoke test (no real auth needed: `none`).
fn fixture_config() -> DevSupportConfig {
    DevSupportConfig::from_toml_str(
        r#"
        [github]
        owner = "acme"
        repo = "widget"
        auth = "none"

        [agent]
        model = "claude-haiku-4-5"
        system_prompt = "You are the dev-support agent for the widget repo."
        tools = ["knowledge_search", "github_search"]
    "#,
    )
    .expect("parse fixture config")
}

#[tokio::test]
async fn ingest_then_grounded_turn_retrieves_repo_fact() {
    let config = fixture_config();

    // 1) Ingest the fixture "repo" through the example's real pipeline.
    let (storage, report) = ingest_into_memory(&fixture_connector(), &config.org_id())
        .await
        .expect("ingest fixture repo");
    assert_eq!(report.documents_pulled, 2, "README + code file");
    assert!(report.chunks_stored >= 2, "report: {report:?}");

    // The store is queryable for the distinctive fact (RAG corpus is real).
    let hits = storage
        .knowledge()
        .query("Frobnicator ring buffer", 3)
        .expect("query knowledge");
    assert!(
        hits.iter().any(|h| h.chunk.contains("42-slot ring buffer")),
        "expected the README fact indexed, got: {hits:?}"
    );

    // 2) Script the LLM: turn 1 calls knowledge_search; turn 2 answers grounded.
    let mock = MockLlmClient::new();
    mock.push_tool_call(
        "call_kb_1",
        "knowledge_search",
        serde_json::json!({ "query": "Frobnicator ring buffer size" }),
    )
    .push_text("The Frobnicator uses a 42-slot ring buffer to batch events before flushing.");

    // A config the runtime can construct from (the mock intercepts every call,
    // so the api_url/key are never used).
    let llm: LlmConfig = LlmConfig::openrouter("not-a-real-key").with_model("claude-haiku-4-5");

    let runtime = DevSupportRuntime::new(
        &config,
        llm,
        tool_github_auth(&config).expect("tool auth"),
        Arc::clone(&storage) as Arc<dyn StorageAdapter>,
    )
    .with_llm_provider(Arc::new(mock.clone()))
    .with_max_iterations(6);

    let outcome = runtime
        .run_turn("How big is the Frobnicator's ring buffer?")
        .await
        .expect("run grounded turn");

    // (a) The agent invoked knowledge_search.
    assert!(
        outcome.invoked_tool("knowledge_search"),
        "expected knowledge_search to run; events: {:?}",
        outcome.events
    );

    // (b) The tool returned the indexed repo fact (retrieval really ran).
    let tool_result = outcome
        .tool_result("knowledge_search")
        .expect("knowledge_search should have completed");
    assert!(
        tool_result.contains("42-slot ring buffer"),
        "tool result should carry the indexed fact, got: {tool_result}"
    );
    assert!(
        tool_result.contains("README.md") || tool_result.contains("acme/widget"),
        "tool result should cite the source, got: {tool_result}"
    );

    // (c) The final grounded answer references the retrieved fact.
    assert!(
        outcome.reply.contains("42"),
        "expected the grounded answer to contain the retrieved 42-slot fact, got: {:?}",
        outcome.reply
    );

    // (d) No live LLM call — exactly two scripted mock calls (search + answer).
    assert_eq!(
        mock.call_count(),
        2,
        "expected exactly 2 mock LLM calls (search decision + grounded answer)"
    );

    // The model was offered BOTH tools on turn 1 (the dev-team wiring).
    let first_call = &mock.calls()[0];
    let offered: Vec<&str> = first_call.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        offered.contains(&"knowledge_search") && offered.contains(&"github_search"),
        "expected both knowledge_search + github_search offered, got: {offered:?}"
    );
}

/// The runtime honors the config's tool selection: with only `knowledge_search`
/// enabled, `github_search` is not offered to the model.
#[tokio::test]
async fn tool_selection_is_respected() {
    let config = DevSupportConfig::from_toml_str(
        r#"
        [github]
        owner = "acme"
        repo = "widget"
        auth = "none"

        [agent]
        tools = ["knowledge_search"]
    "#,
    )
    .expect("parse");

    let (storage, _) = ingest_into_memory(&fixture_connector(), &config.org_id())
        .await
        .expect("ingest");

    let mock = MockLlmClient::new();
    mock.push_text("Hello!");

    let runtime = DevSupportRuntime::new(
        &config,
        LlmConfig::openrouter("x").with_model("claude-haiku-4-5"),
        tool_github_auth(&config).expect("auth"),
        Arc::clone(&storage) as Arc<dyn StorageAdapter>,
    )
    .with_llm_provider(Arc::new(mock.clone()));

    runtime.run_turn("hi").await.expect("turn");

    let offered: Vec<String> = mock.calls()[0]
        .tools
        .iter()
        .map(|t| t.name.clone())
        .collect();
    assert!(offered.contains(&"knowledge_search".to_string()));
    assert!(
        !offered.contains(&"github_search".to_string()),
        "github_search must be absent when not enabled, got: {offered:?}"
    );
}
