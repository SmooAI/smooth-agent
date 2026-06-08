//! Build the DynamoDB [`StorageAdapter`] for the Lambda from resolved config.
//!
//! The OLTP + checkpoint backend is always DynamoDB (the AWS-serverless backend
//! from `docs/STORAGE.md`). The knowledge slice is selected by config:
//!
//! - `SMOOTH_AGENT_VECTOR_BUCKET` set → **Amazon S3 Vectors** (one index per
//!   org), the production dense-retrieval path.
//! - unset → brute-force cosine over DynamoDB (no extra services), useful for
//!   dev/lower environments.
//!
//! Both share the same [`DeterministicEmbedder`] so an org's ingested vectors
//! and query vectors are produced the same way regardless of backend.

use std::sync::Arc;

use anyhow::Result;

use smooth_operator_adapter_dynamodb::{
    DeterministicEmbedder, DynamoDbAdapter, KnowledgeBackend, S3VectorsConfig,
};

use crate::config::LambdaConfig;

/// Construct the DynamoDB-backed adapter for this Lambda.
///
/// Returns the concrete [`DynamoDbAdapter`] in an `Arc` so callers can (a)
/// coerce it to `Arc<dyn StorageAdapter>` for protocol logic AND (b) use its
/// concrete `client()` / `table_name()` for the `$connect`/`$disconnect`
/// connection registry — without a second client.
///
/// Must be called from within a Tokio runtime (the adapter captures the current
/// runtime handle for its sync checkpoint/knowledge bridges).
///
/// # Errors
/// Returns an error if the ambient AWS config / DynamoDB client cannot be built.
pub async fn build_storage(config: &LambdaConfig) -> Result<Arc<DynamoDbAdapter>> {
    // `from_env` reads `SMOOTH_AGENT_DDB_TABLE` (mirrored in `config.table`) and
    // builds the brute-force knowledge default; we then override the knowledge
    // slice with the org partition (and S3 Vectors backend when configured).
    let base = DynamoDbAdapter::from_env(None).await?;

    let backend = match &config.vector_bucket {
        Some(bucket) => KnowledgeBackend::S3Vectors(S3VectorsConfig::new(
            bucket.clone(),
            config.vector_index_prefix.clone(),
        )),
        None => KnowledgeBackend::BruteForce,
    };

    let adapter = base.with_knowledge(
        Arc::new(DeterministicEmbedder::new()),
        config.org_id.clone(),
        backend,
    );

    Ok(Arc::new(adapter))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LambdaConfig;

    #[test]
    fn brute_force_backend_when_no_vector_bucket() {
        let cfg = LambdaConfig {
            table: "t".into(),
            gateway_url: "u".into(),
            gateway_key: None,
            model: "m".into(),
            max_iterations: 1,
            max_tokens: 1,
            org_id: "org".into(),
            vector_bucket: None,
            vector_index_prefix: "idx".into(),
        };
        // Pure selection logic — no AWS calls.
        assert!(matches!(
            match &cfg.vector_bucket {
                Some(b) => KnowledgeBackend::S3Vectors(S3VectorsConfig::new(
                    b.clone(),
                    cfg.vector_index_prefix.clone()
                )),
                None => KnowledgeBackend::BruteForce,
            },
            KnowledgeBackend::BruteForce
        ));
    }
}
