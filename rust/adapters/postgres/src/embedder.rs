//! Text → vector embedding for the pgvector knowledge base.
//!
//! The [`KnowledgeBase`](smooth_operator::KnowledgeBase) impl needs to turn both
//! ingested documents and query strings into dense vectors. We abstract that
//! behind the [`Embedder`] trait so the storage layer never hardcodes a provider:
//!
//! - [`DeterministicEmbedder`] — the **default**. A stable hash-based
//!   pseudo-embedding (no network), L2-normalized, so conformance tests are
//!   reproducible with zero API calls and zero cost. Dimension is configurable;
//!   the Postgres schema defaults to **1024** (mirrors smooai's
//!   `knowledge_vectors embedding vector(1024)`, Voyage `voyage-3-large` shape).
//! - [`GatewayEmbedder`] — optional, only wired when explicitly configured.
//!   Calls an OpenAI-compatible `/v1/embeddings` endpoint (the SmooAI LiteLLM
//!   gateway). `text-embedding-3-small` returns **1536** dims, so if you use the
//!   gateway you must build the adapter with `dim = 1536` to match the column.
//!
//! ## Dimension decision
//!
//! Voyage (`voyage-3-large`, 1024-d) is the production north-star (it backs
//! smooai's `knowledge_vectors`), but Voyage is *not* exposed on the LiteLLM
//! gateway. The gateway does expose OpenAI `text-embedding-3-small` (1536-d).
//! Rather than couple the column width to whichever embedder happens to be
//! configured, the vector dimension is a first-class adapter parameter:
//!
//! | Embedder                | Dim  | Use                              |
//! | ----------------------- | ---- | -------------------------------- |
//! | `DeterministicEmbedder` | 1024 | tests / default (Voyage-shaped)  |
//! | `GatewayEmbedder`       | 1536 | live `text-embedding-3-small`    |
//!
//! The `vector(N)` column and the HNSW index are created at `init` time using
//! the adapter's configured dimension, so dense retrieval is always
//! dimension-consistent.

use anyhow::{anyhow, Result};
use async_trait::async_trait;

/// Default embedding dimension (Voyage `voyage-3-large` shape; mirrors
/// smooai's `knowledge_vectors embedding vector(1024)`).
pub const DEFAULT_EMBEDDING_DIM: usize = 1024;

/// Dimension returned by OpenAI `text-embedding-3-small`.
pub const OPENAI_SMALL_EMBEDDING_DIM: usize = 1536;

/// Whether an embedding is for a document being stored or a search query.
///
/// Voyage and most modern embedding models distinguish the two (asymmetric
/// retrieval). The deterministic embedder ignores it; the gateway embedder maps
/// it onto the OpenAI request unchanged (OpenAI ignores it, but the parameter
/// keeps the seam honest for when a Voyage-native gateway lands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputType {
    /// Embedding a corpus document for storage.
    Document,
    /// Embedding a user query for retrieval.
    Query,
}

/// Turn text into dense vectors. Implementations must return one vector per
/// input string, each of length [`Embedder::dim`].
#[async_trait]
pub trait Embedder: Send + Sync {
    /// The fixed output dimension. Must equal the `vector(N)` column width.
    fn dim(&self) -> usize;

    /// Embed a batch of texts. Returns `texts.len()` vectors, each `dim()` long.
    async fn embed(&self, texts: &[String], input_type: InputType) -> Result<Vec<Vec<f32>>>;
}

/// Deterministic, network-free pseudo-embedder.
///
/// Produces a stable vector from the text via a token-hashing bag-of-words
/// projection, then L2-normalizes it so cosine distance is well-behaved. Same
/// text → same vector, always. This makes pgvector retrieval tests reproducible
/// without any external service: a document and a query that share salient
/// tokens land close together in the projected space.
#[derive(Debug, Clone)]
pub struct DeterministicEmbedder {
    dim: usize,
}

impl DeterministicEmbedder {
    /// Build with the [`DEFAULT_EMBEDDING_DIM`] (1024).
    #[must_use]
    pub fn new() -> Self {
        Self {
            dim: DEFAULT_EMBEDDING_DIM,
        }
    }

    /// Build with a custom dimension (must match the adapter's `vector(N)`).
    #[must_use]
    pub fn with_dim(dim: usize) -> Self {
        Self { dim }
    }

    /// FNV-1a hash of a token — cheap and stable across runs/platforms.
    fn hash_token(token: &str) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in token.bytes() {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    /// Project one text into a normalized vector of `self.dim` floats.
    fn embed_one(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0_f32; self.dim];
        let lower = text.to_lowercase();
        let tokens: Vec<&str> = lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .collect();

        for token in tokens {
            let h = Self::hash_token(token);
            // Two hashed buckets per token with deterministic signs spreads the
            // signal so distinct tokens rarely fully collide.
            let idx_a = (h % self.dim as u64) as usize;
            let idx_b = ((h >> 32) % self.dim as u64) as usize;
            let sign_a = if (h & 1) == 0 { 1.0 } else { -1.0 };
            let sign_b = if (h & 2) == 0 { 1.0 } else { -1.0 };
            v[idx_a] += sign_a;
            v[idx_b] += sign_b;
        }

        // L2-normalize so all vectors live on the unit sphere (cosine == dot).
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    }
}

impl Default for DeterministicEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Embedder for DeterministicEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    async fn embed(&self, texts: &[String], _input_type: InputType) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.embed_one(t)).collect())
    }
}

/// OpenAI-compatible `/v1/embeddings` embedder (the SmooAI LiteLLM gateway).
///
/// Only used when explicitly configured. Reads the endpoint from
/// `SMOOAI_GATEWAY_URL` and the key from `SMOOAI_GATEWAY_KEY` (or pass them in).
/// The default model is `text-embedding-3-small` (1536-d) — set the adapter
/// dimension to [`OPENAI_SMALL_EMBEDDING_DIM`] when using it.
#[derive(Clone)]
pub struct GatewayEmbedder {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    dim: usize,
}

impl GatewayEmbedder {
    /// Build from explicit config.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        dim: usize,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            dim,
        }
    }

    /// Build from `SMOOAI_GATEWAY_URL` + `SMOOAI_GATEWAY_KEY`, defaulting the
    /// model to `text-embedding-3-small` and the dimension to 1536.
    ///
    /// # Errors
    /// Returns an error if either environment variable is unset.
    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("SMOOAI_GATEWAY_URL")
            .map_err(|_| anyhow!("SMOOAI_GATEWAY_URL is not set"))?;
        let api_key = std::env::var("SMOOAI_GATEWAY_KEY")
            .map_err(|_| anyhow!("SMOOAI_GATEWAY_KEY is not set"))?;
        Ok(Self::new(
            base_url,
            api_key,
            "text-embedding-3-small",
            OPENAI_SMALL_EMBEDDING_DIM,
        ))
    }
}

#[async_trait]
impl Embedder for GatewayEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    async fn embed(&self, texts: &[String], _input_type: InputType) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        // Trim a trailing slash so `{base}/v1/embeddings` is well-formed whether
        // the configured URL ends in `/` or not.
        let url = format!("{}/v1/embeddings", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({ "model": self.model, "input": texts });

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
            return Err(anyhow!("embeddings request failed ({status}): {text}"));
        }

        #[derive(serde::Deserialize)]
        struct EmbeddingData {
            embedding: Vec<f32>,
            index: usize,
        }
        #[derive(serde::Deserialize)]
        struct EmbeddingResponse {
            data: Vec<EmbeddingData>,
        }

        let mut parsed: EmbeddingResponse = resp.json().await?;
        // OpenAI returns results in request order but documents `index`; sort to
        // be safe, then validate the dimension matches the column.
        parsed.data.sort_by_key(|d| d.index);
        let out: Vec<Vec<f32>> = parsed.data.into_iter().map(|d| d.embedding).collect();

        if out.len() != texts.len() {
            return Err(anyhow!(
                "embeddings count mismatch: got {} for {} inputs",
                out.len(),
                texts.len()
            ));
        }
        for (i, v) in out.iter().enumerate() {
            if v.len() != self.dim {
                return Err(anyhow!(
                    "embedding {i} has dim {} but adapter expects {}",
                    v.len(),
                    self.dim
                ));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deterministic_is_stable_and_normalized() {
        let e = DeterministicEmbedder::new();
        let a = e
            .embed(&["hello world".to_string()], InputType::Document)
            .await
            .unwrap();
        let b = e
            .embed(&["hello world".to_string()], InputType::Query)
            .await
            .unwrap();
        assert_eq!(a[0].len(), DEFAULT_EMBEDDING_DIM);
        assert_eq!(a, b, "same text must yield the same vector");
        let norm: f32 = a[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "expected unit norm, got {norm}");
    }

    #[tokio::test]
    async fn deterministic_similar_text_is_closer() {
        let e = DeterministicEmbedder::new();
        let vecs = e
            .embed(
                &[
                    "the quick brown fox jumps".to_string(),
                    "the quick brown fox leaps".to_string(),
                    "completely unrelated banana finance report".to_string(),
                ],
                InputType::Document,
            )
            .await
            .unwrap();
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let close = dot(&vecs[0], &vecs[1]);
        let far = dot(&vecs[0], &vecs[2]);
        assert!(
            close > far,
            "shared-token texts should be more similar ({close} vs {far})"
        );
    }

    #[tokio::test]
    async fn custom_dim_respected() {
        let e = DeterministicEmbedder::with_dim(1536);
        let v = e
            .embed(&["x".to_string()], InputType::Document)
            .await
            .unwrap();
        assert_eq!(v[0].len(), 1536);
    }
}
