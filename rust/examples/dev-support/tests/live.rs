//! Gated live test: one real turn through the dev-support runtime against the
//! live `llm.smoo.ai` gateway.
//!
//! ## Gating (safe to commit, safe in CI)
//!
//! A no-op unless BOTH are set:
//!   - `SMOOTH_AGENT_E2E=1`        — explicit opt-in
//!   - `SMOOAI_GATEWAY_KEY=<key>`  — the gateway key (never hardcoded)
//!
//! With `GITHUB_TOKEN` also set, it ingests a tiny real public repo; otherwise it
//! seeds the knowledge store directly (so the LLM + grounding path is still
//! exercised without GitHub). When the gate is missing it prints a skip notice
//! and returns — `cargo test` with no env stays green.
//!
//! ```sh
//! export SMOOAI_GATEWAY_KEY=…
//! export SMOOTH_AGENT_E2E=1
//! cargo test -p smooai-smooth-operator-example-dev-support --test live \
//!   -- --nocapture --test-threads=1
//! ```

use std::sync::Arc;

use dev_support::config::{DevSupportConfig, DEFAULT_GATEWAY_URL};
use dev_support::ingest::{build_connector, ingest_into_memory};
use dev_support::runtime::{gateway_llm_config, tool_github_auth, DevSupportRuntime};

use smooth_operator::StorageAdapter;
use smooth_operator_adapter_memory::InMemoryStorageAdapter;
use smooth_operator_core::{Document, DocumentType};

const DISTINCTIVE_FACT: &str = "the Frobnicator uses a 73-slot ring buffer";

/// Returns the gateway key from env, or `None` (with a skip notice). Never
/// prints the key.
fn gate(test_name: &str) -> Option<String> {
    if std::env::var("SMOOTH_AGENT_E2E").as_deref() != Ok("1") {
        eprintln!("[skip] {test_name}: SMOOTH_AGENT_E2E != \"1\" — skipping live test");
        return None;
    }
    match std::env::var("SMOOAI_GATEWAY_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => {
            eprintln!("[skip] {test_name}: SMOOAI_GATEWAY_KEY unset/empty — skipping live test");
            None
        }
    }
}

#[tokio::test]
async fn live_grounded_turn_against_gateway() {
    let Some(key) = gate("live_grounded_turn_against_gateway") else {
        return;
    };

    // Use a small public repo when a token is present; otherwise seed directly.
    let config = DevSupportConfig::from_toml_str(
        r#"
        [github]
        owner = "rust-lang"
        repo = "rfcs"
        auth = "none"

        [agent]
        tools = ["knowledge_search", "github_search"]
        "#,
    )
    .expect("parse config");

    let storage: Arc<dyn StorageAdapter> = if std::env::var("GITHUB_TOKEN").is_ok() {
        // Real ingest of a tiny public repo (subject to anonymous rate limits if
        // the token is empty, but its presence opts us into the live GitHub path).
        let connector = build_connector(&config).expect("connector");
        match ingest_into_memory(&connector, &config.org_id()).await {
            Ok((storage, report)) => {
                eprintln!(
                    "[live] ingested {} docs / {} chunks from {}",
                    report.documents_pulled,
                    report.chunks_stored,
                    config.repo_slug()
                );
                storage
            }
            Err(e) => {
                eprintln!("[live] GitHub ingest failed ({e:#}); falling back to seeded knowledge");
                seeded_storage()
            }
        }
    } else {
        eprintln!(
            "[live] GITHUB_TOKEN unset — seeding knowledge directly (skipping GitHub ingest)"
        );
        seeded_storage()
    };

    let gateway_url =
        std::env::var("SMOOAI_GATEWAY_URL").unwrap_or_else(|_| DEFAULT_GATEWAY_URL.to_string());
    let llm = gateway_llm_config(&config.agent.model, key, gateway_url, 512);

    let runtime = DevSupportRuntime::new(
        &config,
        llm,
        tool_github_auth(&config).expect("tool auth"),
        Arc::clone(&storage),
    )
    .with_max_iterations(6);

    let question = if storage_has_seeded_fact(&storage) {
        "How many slots does the Frobnicator's ring buffer have? Search the knowledge base."
    } else {
        "What is this repository about? Search the knowledge base and answer briefly."
    };

    let outcome = runtime.run_turn(question).await.expect("live run_turn");
    eprintln!("[live] reply: {:?}", outcome.reply);
    eprintln!("[live] tools used: {:?}", outcome.tools_used());

    assert!(
        !outcome.reply.trim().is_empty(),
        "expected a non-empty grounded answer from the live model"
    );
    if storage_has_seeded_fact(&storage) {
        assert!(
            outcome.reply.contains("73"),
            "expected the grounded answer to contain the seeded 73-slot fact, got: {:?}",
            outcome.reply
        );
    }
}

/// An in-memory store seeded with the distinctive fact directly (no GitHub).
fn seeded_storage() -> Arc<dyn StorageAdapter> {
    let storage = Arc::new(InMemoryStorageAdapter::new());
    storage
        .knowledge()
        .ingest(Document::new(
            format!("Frobnicator internals: {DISTINCTIVE_FACT} to batch events before flushing."),
            "README.md",
            DocumentType::Documentation,
        ))
        .expect("seed knowledge");
    storage
}

/// Whether the store carries the seeded 73-slot fact (vs. a real repo ingest).
fn storage_has_seeded_fact(storage: &Arc<dyn StorageAdapter>) -> bool {
    storage
        .knowledge()
        .query("Frobnicator ring buffer", 1)
        .map(|h| h.iter().any(|r| r.chunk.contains("73-slot ring buffer")))
        .unwrap_or(false)
}
