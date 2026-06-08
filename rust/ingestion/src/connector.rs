//! The [`Connector`] seam and its raw-document payload.
//!
//! A connector pulls documents from an external source (a file tree, a web
//! page, a SaaS API) into a normalized [`RawDocument`] the ingestion pipeline
//! can chunk, embed, and store. This is the smooth-operator analog of
//! Onyx's 58+ connectors (Onyx-gap G1): one trait, a credential-free
//! [`MockConnector`] for the contract test, and real connectors (file/web)
//! whose live paths are gated behind an external-dependency flag (G9).

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

/// A point in time used to scope an incremental pull (`pull(since)`).
pub type Timestamp = DateTime<Utc>;

/// A source document as pulled by a [`Connector`], before chunking/embedding.
///
/// `id` is the connector-stable identity of the source document (a file path, a
/// URL, an external record id). The pipeline keys idempotency on `(id, content
/// hash)`, so a connector that returns a stable `id` for the same logical
/// document lets re-ingests skip unchanged content.
#[derive(Debug, Clone)]
pub struct RawDocument {
    /// Connector-stable identity of the source document.
    pub id: String,
    /// Human/source label for the document's origin (e.g. `"file"`, `"web"`).
    pub source: String,
    /// Optional human title (carried into chunk metadata when present).
    pub title: Option<String>,
    /// The document's textual content (already HTML-stripped for the web case).
    pub content: String,
    /// Arbitrary source metadata, propagated onto every chunk.
    pub metadata: HashMap<String, String>,
    /// Optional access-control labels (e.g. group/user ids that may read this
    /// document). Carried through for a future ACL-filtered retrieval (G3); the
    /// current pipeline propagates but does not yet enforce them.
    pub acl: Option<Vec<String>>,
}

impl RawDocument {
    /// Build a raw document from its stable `id`, `source`, and `content`.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        source: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            title: None,
            content: content.into(),
            metadata: HashMap::new(),
            acl: None,
        }
    }

    /// Attach a human title (builder).
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Add a metadata key/value (builder).
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Attach access-control labels (builder).
    #[must_use]
    pub fn with_acl(mut self, acl: Vec<String>) -> Self {
        self.acl = Some(acl);
        self
    }
}

/// A source of [`RawDocument`]s.
///
/// `pull(since)` returns every document the connector currently exposes, or —
/// when `since` is `Some` and the source supports incremental sync — only those
/// changed at/after that timestamp. Connectors that cannot do incremental sync
/// ignore `since` and return the full set (the pipeline's `(id, hash)`
/// idempotency keeps re-ingests cheap regardless).
#[async_trait]
pub trait Connector: Send + Sync {
    /// A short label for this connector kind (e.g. `"file"`, `"web"`, `"mock"`).
    fn name(&self) -> &str;

    /// Pull documents, optionally only those changed since `since`.
    ///
    /// # Errors
    /// Returns an error if the source cannot be read.
    async fn pull(&self, since: Option<Timestamp>) -> Result<Vec<RawDocument>>;
}

/// A fixed-payload connector for tests — yields the documents it was built with.
///
/// The credential-free fixture behind the ingestion contract test (G9: the
/// `unit` tier that runs on every PR).
pub struct MockConnector {
    docs: Vec<RawDocument>,
}

impl MockConnector {
    /// Build a connector that always yields `docs`.
    #[must_use]
    pub fn new(docs: Vec<RawDocument>) -> Self {
        Self { docs }
    }
}

#[async_trait]
impl Connector for MockConnector {
    fn name(&self) -> &str {
        "mock"
    }

    async fn pull(&self, _since: Option<Timestamp>) -> Result<Vec<RawDocument>> {
        Ok(self.docs.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_connector_yields_its_docs() {
        let connector = MockConnector::new(vec![
            RawDocument::new("a", "mock", "alpha").with_title("A"),
            RawDocument::new("b", "mock", "beta").with_metadata("k", "v"),
        ]);
        assert_eq!(connector.name(), "mock");
        let docs = connector.pull(None).await.unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].id, "a");
        assert_eq!(docs[0].title.as_deref(), Some("A"));
        assert_eq!(docs[1].metadata.get("k").map(String::as_str), Some("v"));
    }
}
