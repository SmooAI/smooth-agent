//! Text → vector embedding seam for the ingestion pipeline.
//!
//! This mirrors the **same `Embedder` seam** the Postgres adapter defines
//! (`adapters/postgres/src/embedder.rs`): a provider-agnostic trait plus a
//! deterministic, network-free default so the ingestion pipeline (and its
//! contract test) embeds chunks reproducibly with zero API calls and zero cost.
//!
//! ## Why a copy and not a shared import?
//!
//! The Postgres adapter's `Embedder` is the production north-star, but pulling
//! it in here would drag `deadpool-postgres` / `tokio-postgres` (and a
//! `testcontainers` dev-dep) into a crate that has nothing to do with Postgres.
//! The trait is a tiny, stable contract, so we keep a minimal copy here and hold
//! the two byte-identical in `DeterministicEmbedder` (same FNV-1a hashing, same
//! L2 normalization, same 1024-d default → same vectors). `docs/INGESTION.md`
//! records the relationship; if the seam ever moves to a shared crate, both
//! sites adopt it. The retrieval-relevant invariant — same text → same vector,
//! shared-token texts land closer — is asserted in `tests`.

use anyhow::Result;
use async_trait::async_trait;

/// Default embedding dimension (Voyage `voyage-3-large` shape; mirrors smooai's
/// `knowledge_vectors embedding vector(1024)` and the Postgres adapter default).
pub const DEFAULT_EMBEDDING_DIM: usize = 1024;

/// Whether an embedding is for a document being stored or a search query.
///
/// Modern embedding models distinguish the two (asymmetric retrieval). The
/// deterministic embedder ignores it; the parameter keeps the seam honest for a
/// provider-backed embedder that respects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputType {
    /// Embedding a corpus document for storage.
    Document,
    /// Embedding a user query for retrieval.
    Query,
}

/// Turn text into dense vectors. Implementations return one vector per input
/// string, each of length [`Embedder::dim`].
#[async_trait]
pub trait Embedder: Send + Sync {
    /// The fixed output dimension.
    fn dim(&self) -> usize;

    /// Embed a batch of texts. Returns `texts.len()` vectors, each `dim()` long.
    ///
    /// # Errors
    /// Returns an error if the backing embedding service fails.
    async fn embed(&self, texts: &[String], input_type: InputType) -> Result<Vec<Vec<f32>>>;
}

/// Deterministic, network-free pseudo-embedder.
///
/// Token-hashing bag-of-words projection, L2-normalized. Same text → same
/// vector, always. Byte-for-byte identical to the Postgres adapter's
/// `DeterministicEmbedder` (FNV-1a, two signed buckets per token, unit norm), so
/// vectors are consistent across the two crates.
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

    /// Build with a custom dimension.
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
        let tokens = lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty());

        for token in tokens {
            let h = Self::hash_token(token);
            let idx_a = (h % self.dim as u64) as usize;
            let idx_b = ((h >> 32) % self.dim as u64) as usize;
            let sign_a = if (h & 1) == 0 { 1.0 } else { -1.0 };
            let sign_b = if (h & 2) == 0 { 1.0 } else { -1.0 };
            v[idx_a] += sign_a;
            v[idx_b] += sign_b;
        }

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
    async fn shared_token_text_is_closer() {
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
        assert!(
            dot(&vecs[0], &vecs[1]) > dot(&vecs[0], &vecs[2]),
            "shared-token texts should be more similar"
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
