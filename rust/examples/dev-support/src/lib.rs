//! # dev-support — a dev-team knowledge & support agent over your GitHub repo
//!
//! The showcase recipe: ingest a GitHub repo's prose, code, and issues into a
//! smooth-operator knowledge store, then chat — grounded in the repo, with a
//! live `github_search` for anything newer than the last ingest.
//!
//! This library crate is the example's testable core (config → ingest →
//! runtime); the `dev-support` binary ([`main`](../main.rs)) is a thin CLI over
//! it. Splitting it this way means the whole pipeline is exercised by a smoke
//! test with **no binary, no network, no real GitHub** — see
//! `tests/smoke.rs`.
//!
//! ```text
//!   GitHub repo ──connector──▶ ingest (chunk→embed→store) ──▶ knowledge store
//!                                                                    │
//!   user question ──▶ DevSupportRuntime ──┬─ knowledge_search (indexed snapshot)
//!                                          └─ github_search    (live lookups)
//!                                                     │
//!                                                     ▼
//!                                            grounded answer
//! ```
//!
//! ## Modules
//! - [`config`] — parse `dev-support.toml` (+ `$GITHUB_TOKEN` / `$SMOOAI_GATEWAY_KEY`).
//! - [`ingest`] — build the connector + run the ingestion pipeline.
//! - [`runtime`] — [`DevSupportRuntime`](runtime::DevSupportRuntime): the two
//!   tools + the gateway wired onto a real smooth-operator `Agent`.

pub mod config;
pub mod ingest;
pub mod runtime;

pub use config::{AuthMode, DevSupportConfig, IncludeConfig, ToolName};
pub use ingest::{build_connector, ingest_into, ingest_into_memory};
pub use runtime::{gateway_llm_config, tool_github_auth, DevSupportRuntime, TurnOutcome};
