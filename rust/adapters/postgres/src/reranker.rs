//! Adapter-specific reranker: the live [`GatewayReranker`] (feature gap G8).
//!
//! The provider-agnostic [`Reranker`] trait, the identity [`NoopReranker`], and
//! the network-free [`LexicalReranker`] all live in
//! [`smooth_operator::rerank`] — the shared home so the retrieval path can swap a
//! reranker in without depending on any paid API. This module holds only the
//! adapter-specific [`GatewayReranker`]: a cross-encoder reranker behind the
//! SmooAI LiteLLM gateway's Cohere/Voyage-style `/v1/rerank` endpoint, exactly as
//! [`GatewayEmbedder`](crate::GatewayEmbedder) holds the live `/v1/embeddings`
//! client. It drags `reqwest` and so lives here rather than in `core`.
//!
//! ## Endpoint shape
//!
//! The gateway exposes a Cohere/Voyage-compatible rerank route:
//!
//! ```text
//! POST {base}/v1/rerank
//! { "model": "...", "query": "...", "documents": ["doc0", "doc1", ...],
//!   "top_n": K }
//! → { "results": [ { "index": <usize>, "relevance_score": <f32> }, ... ] }
//! ```
//!
//! The reranker sends the candidate chunks as `documents`, reads the returned
//! `index → relevance_score`, and reorders the **original** candidates by that
//! score (highest first), truncating to `top_k`.
//!
//! ## Failure is non-fatal (never drop the turn)
//!
//! A reranker is a *quality* stage, not a *correctness* stage: the upstream
//! retrieval already produced a usable, rank-ordered candidate set. So any
//! failure on the rerank call — network error, non-2xx, malformed JSON, an
//! out-of-range index — degrades gracefully to the **input order** (truncated to
//! `top_k`), logging a [`tracing::warn!`]. It never panics and never drops the
//! turn. This mirrors the embedder's "fail loud, keep working" posture, tuned to
//! the fact that an identity reorder is a perfectly safe fallback here.
//!
//! ## Testability seam
//!
//! [`GatewayReranker`] is generic over a [`RerankBackend`] — the thing that turns
//! `(query, documents, top_n)` into `(index, score)` pairs. The production
//! backend is [`HttpRerankBackend`] (the real `/v1/rerank` call). Unit tests
//! inject a stub backend so the reorder/truncate/error-fallback logic is exercised
//! **without touching the network**, exactly how `github_search` tests its
//! `GithubSearchBackend`.

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use smooth_operator::rerank::Reranker;
use smooth_operator_core::KnowledgeResult;

/// Default rerank model requested over the gateway (Cohere-compatible). Distinct
/// from the embedding model and the chat model.
pub const DEFAULT_RERANK_MODEL: &str = "rerank-english-v3.0";

/// One scored candidate returned by a [`RerankBackend`]: the candidate's index in
/// the request `documents` array, and its relevance score against the query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RerankScore {
    /// Index into the `documents` slice the backend was given.
    pub index: usize,
    /// Relevance score; higher is more relevant. Reordering is by this value
    /// descending.
    pub relevance_score: f32,
}

/// A pluggable rerank backend.
///
/// The production [`HttpRerankBackend`] POSTs to the gateway's `/v1/rerank`. Tests
/// inject a stub so the [`GatewayReranker`] reorder/truncate/error-fallback logic
/// runs offline (mirrors `github_search`'s `GithubSearchBackend` seam).
#[async_trait]
pub trait RerankBackend: Send + Sync {
    /// Score `documents` against `query`, returning at most `top_n` `(index,
    /// score)` pairs. Implementations need not sort — [`GatewayReranker`] sorts
    /// the returned scores itself.
    ///
    /// # Errors
    /// Returns an error if the upstream rerank call fails; [`GatewayReranker`]
    /// catches it and falls back to the input order.
    async fn rerank(
        &self,
        query: &str,
        documents: &[String],
        top_n: usize,
    ) -> Result<Vec<RerankScore>>;
}

/// The real backend: a Cohere/Voyage-style `/v1/rerank` call over the gateway.
#[derive(Clone)]
pub struct HttpRerankBackend {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl HttpRerankBackend {
    /// Build from explicit config.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }
}

#[async_trait]
impl RerankBackend for HttpRerankBackend {
    async fn rerank(
        &self,
        query: &str,
        documents: &[String],
        top_n: usize,
    ) -> Result<Vec<RerankScore>> {
        // Trim a trailing slash so `{base}/v1/rerank` is well-formed whether the
        // configured URL ends in `/` or not.
        let url = format!("{}/v1/rerank", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model,
            "query": query,
            "documents": documents,
            "top_n": top_n,
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("rerank request failed ({status}): {text}"));
        }

        #[derive(serde::Deserialize)]
        struct ResultItem {
            index: usize,
            relevance_score: f32,
        }
        #[derive(serde::Deserialize)]
        struct RerankResponse {
            results: Vec<ResultItem>,
        }

        let parsed: RerankResponse = resp.json().await?;
        Ok(parsed
            .results
            .into_iter()
            .map(|r| RerankScore {
                index: r.index,
                relevance_score: r.relevance_score,
            })
            .collect())
    }
}

/// Cross-encoder reranker over the SmooAI gateway's `/v1/rerank` endpoint
/// (feature gap G8).
///
/// Reorders retrieval candidates by a sharp query↔candidate relevance model and
/// truncates to `top_k`. On any backend failure it falls back to the input order
/// (truncated) — a reranker is a quality stage, so an identity reorder is always
/// a safe fallback. Construct with [`from_env`](Self::from_env) for the live
/// gateway, or [`with_backend`](Self::with_backend) to inject a stub in tests.
pub struct GatewayReranker {
    backend: std::sync::Arc<dyn RerankBackend>,
}

impl GatewayReranker {
    /// Build over an explicit backend. Production passes an
    /// [`HttpRerankBackend`]; tests pass a stub.
    #[must_use]
    pub fn with_backend(backend: std::sync::Arc<dyn RerankBackend>) -> Self {
        Self { backend }
    }

    /// Build the live gateway reranker from explicit config.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self::with_backend(std::sync::Arc::new(HttpRerankBackend::new(
            base_url, api_key, model,
        )))
    }

    /// Build from `SMOOAI_GATEWAY_URL` + `SMOOAI_GATEWAY_KEY`, defaulting the
    /// model to [`DEFAULT_RERANK_MODEL`].
    ///
    /// # Errors
    /// Returns an error if either environment variable is unset.
    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("SMOOAI_GATEWAY_URL")
            .map_err(|_| anyhow!("SMOOAI_GATEWAY_URL is not set"))?;
        let api_key = std::env::var("SMOOAI_GATEWAY_KEY")
            .map_err(|_| anyhow!("SMOOAI_GATEWAY_KEY is not set"))?;
        Ok(Self::new(base_url, api_key, DEFAULT_RERANK_MODEL))
    }

    /// Reorder `candidates` by the backend's `(index, score)` pairs.
    ///
    /// Scores are sorted descending; any candidate the backend did NOT score (or
    /// scored with an out-of-range index, which is ignored) keeps a lower priority
    /// than scored ones and retains its upstream order — so a partial response
    /// still degrades sanely rather than dropping candidates.
    fn reorder(
        mut scores: Vec<RerankScore>,
        candidates: Vec<KnowledgeResult>,
        top_k: usize,
    ) -> Vec<KnowledgeResult> {
        let n = candidates.len();
        // Move candidates into Options so we can take() each at most once.
        let mut slots: Vec<Option<KnowledgeResult>> = candidates.into_iter().map(Some).collect();

        // Stable sort by score descending so equal scores keep backend order.
        scores.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut out: Vec<KnowledgeResult> = Vec::with_capacity(top_k.min(n));
        let mut taken = vec![false; n];
        for s in scores {
            if out.len() >= top_k {
                break;
            }
            // Ignore out-of-range indices from a misbehaving backend rather than
            // panicking.
            if s.index < n && !taken[s.index] {
                if let Some(c) = slots[s.index].take() {
                    taken[s.index] = true;
                    out.push(c);
                }
            }
        }
        // Append any unscored candidates (backend returned fewer than n, e.g. a
        // top_n cutoff) in upstream order, until top_k is reached.
        if out.len() < top_k {
            for (i, slot) in slots.iter_mut().enumerate() {
                if out.len() >= top_k {
                    break;
                }
                if !taken[i] {
                    if let Some(c) = slot.take() {
                        out.push(c);
                    }
                }
            }
        }
        out
    }
}

#[async_trait]
impl Reranker for GatewayReranker {
    async fn rerank(
        &self,
        query: &str,
        candidates: Vec<KnowledgeResult>,
        top_k: usize,
    ) -> Vec<KnowledgeResult> {
        if candidates.is_empty() || top_k == 0 {
            return Vec::new();
        }
        let documents: Vec<String> = candidates.iter().map(|c| c.chunk.clone()).collect();

        match self.backend.rerank(query, &documents, top_k).await {
            Ok(scores) => Self::reorder(scores, candidates, top_k),
            Err(e) => {
                // Quality stage: a rerank failure must never drop the turn. Fall
                // back to the upstream order, truncated to top_k.
                tracing::warn!(
                    error = %e,
                    "GatewayReranker call failed; falling back to upstream candidate order"
                );
                let mut fallback = candidates;
                fallback.truncate(top_k);
                fallback
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn result(id: &str, chunk: &str) -> KnowledgeResult {
        KnowledgeResult {
            document_id: id.to_string(),
            chunk: chunk.to_string(),
            score: 0.5,
            source: format!("{id}.md"),
        }
    }

    /// A stub backend that returns caller-supplied scores — no network.
    struct StubBackend {
        scores: Vec<RerankScore>,
    }
    #[async_trait]
    impl RerankBackend for StubBackend {
        async fn rerank(
            &self,
            _query: &str,
            _documents: &[String],
            _top_n: usize,
        ) -> Result<Vec<RerankScore>> {
            Ok(self.scores.clone())
        }
    }

    /// A stub backend that always errors — exercises the graceful fallback.
    struct ErrorBackend;
    #[async_trait]
    impl RerankBackend for ErrorBackend {
        async fn rerank(
            &self,
            _query: &str,
            _documents: &[String],
            _top_n: usize,
        ) -> Result<Vec<RerankScore>> {
            Err(anyhow!("simulated rerank API failure"))
        }
    }

    /// TDD: the highest-relevance candidate is seeded LAST; the GatewayReranker
    /// must promote it to the front using the backend's scores.
    #[tokio::test]
    async fn gateway_reranker_reorders_by_relevance() {
        // Upstream order: index 0 weak, index 1 medium, index 2 strong.
        let candidates = vec![
            result("shipping", "shipping takes 5-7 days"),
            result("warranty", "warranty is one year"),
            result("returns", "30 day refund window"),
        ];
        // Backend says index 2 is most relevant, then 1, then 0.
        let scores = vec![
            RerankScore {
                index: 0,
                relevance_score: 0.1,
            },
            RerankScore {
                index: 1,
                relevance_score: 0.4,
            },
            RerankScore {
                index: 2,
                relevance_score: 0.95,
            },
        ];
        let reranker = GatewayReranker::with_backend(Arc::new(StubBackend { scores }));
        let out = reranker.rerank("refund returns", candidates, 3).await;

        assert_eq!(
            out.iter()
                .map(|r| r.document_id.as_str())
                .collect::<Vec<_>>(),
            vec!["returns", "warranty", "shipping"],
            "candidates must be reordered by descending relevance score"
        );
    }

    #[tokio::test]
    async fn gateway_reranker_truncates_to_top_k() {
        let candidates = vec![
            result("a", "alpha"),
            result("b", "beta"),
            result("c", "gamma"),
        ];
        let scores = vec![
            RerankScore {
                index: 2,
                relevance_score: 0.9,
            },
            RerankScore {
                index: 0,
                relevance_score: 0.5,
            },
            RerankScore {
                index: 1,
                relevance_score: 0.1,
            },
        ];
        let reranker = GatewayReranker::with_backend(Arc::new(StubBackend { scores }));
        let out = reranker.rerank("q", candidates, 1).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].document_id, "c", "top_k=1 keeps only the best");
    }

    /// API error → input order preserved (truncated), no panic, no drop.
    #[tokio::test]
    async fn gateway_reranker_error_falls_back_to_input_order() {
        let candidates = vec![
            result("first", "one"),
            result("second", "two"),
            result("third", "three"),
        ];
        let reranker = GatewayReranker::with_backend(Arc::new(ErrorBackend));
        let out = reranker.rerank("anything", candidates, 2).await;

        assert_eq!(out.len(), 2, "fallback truncates to top_k");
        assert_eq!(
            out.iter()
                .map(|r| r.document_id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"],
            "on error the upstream order is preserved"
        );
    }

    /// A partial backend response (fewer scores than candidates) still returns
    /// the unscored candidates in upstream order rather than dropping them.
    #[tokio::test]
    async fn gateway_reranker_partial_scores_appends_unscored_in_order() {
        let candidates = vec![result("a", "aaa"), result("b", "bbb"), result("c", "ccc")];
        // Backend only scored index 2.
        let scores = vec![RerankScore {
            index: 2,
            relevance_score: 0.9,
        }];
        let reranker = GatewayReranker::with_backend(Arc::new(StubBackend { scores }));
        let out = reranker.rerank("q", candidates, 3).await;
        assert_eq!(
            out.iter()
                .map(|r| r.document_id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "a", "b"],
            "scored candidate first, then unscored in upstream order"
        );
    }

    /// An out-of-range index from a misbehaving backend is ignored, not panicked.
    #[tokio::test]
    async fn gateway_reranker_ignores_out_of_range_index() {
        let candidates = vec![result("a", "aaa"), result("b", "bbb")];
        let scores = vec![
            RerankScore {
                index: 99, // out of range
                relevance_score: 0.99,
            },
            RerankScore {
                index: 1,
                relevance_score: 0.5,
            },
        ];
        let reranker = GatewayReranker::with_backend(Arc::new(StubBackend { scores }));
        let out = reranker.rerank("q", candidates, 2).await;
        // index 99 ignored; index 1 promoted, then unscored index 0 appended.
        assert_eq!(
            out.iter()
                .map(|r| r.document_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
    }

    #[tokio::test]
    async fn gateway_reranker_empty_candidates_yields_empty() {
        let reranker = GatewayReranker::with_backend(Arc::new(StubBackend { scores: vec![] }));
        let out = reranker.rerank("q", vec![], 3).await;
        assert!(out.is_empty());
    }

    /// Live rerank against the real gateway — only with `SMOOTH_AGENT_E2E=1` and a
    /// `SMOOAI_GATEWAY_KEY`. Ignored by default (network + creds + the gateway
    /// must actually expose a `/v1/rerank` route, which is not guaranteed). Run:
    /// `SMOOTH_AGENT_E2E=1 cargo test -p smooai-smooth-operator-adapter-postgres \
    ///    reranker::tests::live_rerank -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "network + creds: gated on SMOOTH_AGENT_E2E=1 and a /v1/rerank route"]
    async fn live_rerank() {
        if std::env::var("SMOOTH_AGENT_E2E").as_deref() != Ok("1") {
            eprintln!("skipping live rerank: set SMOOTH_AGENT_E2E=1 to run");
            return;
        }
        let Ok(reranker) = GatewayReranker::from_env() else {
            eprintln!("skipping live rerank: SMOOAI_GATEWAY_URL / SMOOAI_GATEWAY_KEY not set");
            return;
        };
        let candidates = vec![
            result("shipping", "Standard shipping takes 5 to 7 business days."),
            result("warranty", "Warranty claims must be filed within one year."),
            result(
                "returns",
                "Our return policy: refunds within the 30 day window.",
            ),
        ];
        let out = reranker
            .rerank("how do refunds and returns work", candidates, 3)
            .await;
        eprintln!(
            "live rerank order: {:?}",
            out.iter()
                .map(|r| r.document_id.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(out.len(), 3, "live rerank should return all 3 reordered");
    }
}
