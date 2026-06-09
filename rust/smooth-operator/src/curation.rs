//! Document sets, curation boosting, and query-time metadata filters
//! (feature gap, Phase 11 — "Document sets / curation / boosting").
//!
//! Curation lets a user (a) group documents into named **document sets** so a
//! query can be scoped to "only the dev-support repo" or "only the HR handbook",
//! (b) **boost** canonical documents (the README, the policy of record) so they
//! outrank merely-similar matches, and (c) filter retrieval by arbitrary
//! **metadata equality** ("only prose, not code"). This module adds all three to
//! smooth-operator, as a retrieval-time filter applied in *our* layer — exactly
//! like [`AclKnowledgeStore`](crate::access_control::AclKnowledgeStore).
//!
//! ## Why enforcement lives in our layer (same reason as ACL)
//!
//! The engine's [`KnowledgeBase`](smooth_operator_core::KnowledgeBase) trait is
//! upstream and read-only to us; its `query` returns a
//! [`KnowledgeResult`](smooth_operator_core::KnowledgeResult) carrying only
//! `document_id` / `chunk` / `score` / `source` — **not** the stored metadata —
//! and the in-memory backend drops document metadata on ingest entirely. So we
//! cannot read a document's set membership / boost / metadata back out of a
//! query result. Instead this module, mirroring [`access_control`](crate::access_control):
//!
//! 1. Records each document's [`DocMeta`] (parsed from the metadata stamped at
//!    ingest — see the [metadata convention](#metadata-convention)) into a side
//!    table the [`CuratedKnowledgeStore`] owns, while forwarding the document
//!    unchanged to the inner backend.
//! 2. **Filters + boosts at read**: a reader bound to a [`RetrievalFilter`]
//!    over-fetches from the inner backend, drops documents not in the requested
//!    sets / not matching the metadata equalities, multiplies each surviving
//!    result's score by its [`boost`](DocMeta::boost), **re-sorts** by the boosted
//!    score, and truncates to the requested `K`.
//!
//! ## Metadata convention
//!
//! Stamped onto [`Document::metadata`](smooth_operator_core::Document) at ingest
//! (the ingestion pipeline writes these; a connector/ingest config supplies the
//! values):
//!
//! - **`document_set`** — set membership. **Multi-valued via a comma list**:
//!   `"alpha"` is one set, `"alpha,beta"` is both. Names are trimmed of
//!   surrounding whitespace; empty names are dropped. A document with no
//!   `document_set` belongs to no named set (so a set-scoped query never
//!   surfaces it, but an unscoped query — `document_sets: None` — still does).
//! - **`boost`** — a parsed `f32` multiplier on the similarity score, default
//!   **1.0**. Absent or malformed (`"abc"`, `""`, `NaN`, non-finite) ⇒ `1.0`, so
//!   a bad stamp can never silently zero out or explode a document's ranking.
//!   Negative boosts are clamped to `0.0` (a curator can bury a doc, never invert
//!   ordering). `boost > 1.0` promotes; `0.0 ≤ boost < 1.0` demotes.
//!
//! ## Composition with ACL
//!
//! A [`CuratedKnowledgeStore`] also records [`DocAcl`](crate::access_control::DocAcl)s
//! at ingest (same `acl_v2` key) and its reader takes an
//! [`AccessContext`](crate::access_control::AccessContext) alongside the
//! [`RetrievalFilter`]. **Both filters apply (logical AND)**: a result is
//! returned only if the requester is entitled to it (ACL) *and* it is in the
//! requested sets *and* it matches the metadata equalities. ACL is checked first
//! (a curation filter must never widen what a requester can see).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use smooth_operator_core::{Document, KnowledgeBase, KnowledgeResult};

use crate::access_control::{AccessContext, DocAcl};

/// Over-fetch multiplier: the inner backend is queried for `limit * this` (with
/// a floor) candidates so that, after dropping non-matching documents and
/// re-ranking by boost, the post-filter top-K is still full whenever enough
/// matching documents exist. Mirrors the ACL reader's over-fetch.
const OVERFETCH_FACTOR: usize = 5;

/// Lower bound on the candidate pool, so a small `limit` still over-fetches
/// enough to survive filtering + boost re-ranking.
const OVERFETCH_FLOOR: usize = 20;

/// The default boost applied to a document with no (or a malformed) `boost`
/// metadata value: a no-op multiplier that preserves the raw similarity score.
pub const DEFAULT_BOOST: f32 = 1.0;

/// Curation metadata recorded for a stored document: its document-set
/// membership and its retrieval boost, plus the raw metadata map so the
/// [`RetrievalFilter`]'s `metadata_eq` equalities can be evaluated at read.
///
/// Parsed from a [`Document`]'s metadata at ingest by [`DocMeta::from_document`].
#[derive(Debug, Clone, PartialEq)]
pub struct DocMeta {
    /// The named sets this document belongs to (parsed from the comma-separated
    /// `document_set` metadata value; empty when the document is in no set).
    pub document_sets: Vec<String>,
    /// The retrieval boost multiplier (parsed from the `boost` metadata value;
    /// [`DEFAULT_BOOST`] when absent/malformed). Clamped to `≥ 0.0`.
    pub boost: f32,
    /// The full stamped metadata map, retained so `metadata_eq` filters can test
    /// arbitrary key/value equalities against it.
    pub metadata: HashMap<String, String>,
}

impl DocMeta {
    /// The document-metadata key under which set membership is stamped. The
    /// value is a **comma-separated** list (e.g. `"alpha"` or `"alpha,beta"`).
    pub const DOCUMENT_SET_KEY: &'static str = "document_set";

    /// The document-metadata key under which the numeric boost is stamped (a
    /// stringified `f32`, e.g. `"3.0"`).
    pub const BOOST_KEY: &'static str = "boost";

    /// Parse the document-set list from a `document_set` metadata value:
    /// comma-split, trim each name, drop empties. Returns an empty vec when the
    /// key is absent or holds only whitespace/commas.
    #[must_use]
    pub fn parse_sets(metadata: &HashMap<String, String>) -> Vec<String> {
        metadata
            .get(Self::DOCUMENT_SET_KEY)
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Parse the boost from a `boost` metadata value. Absent / unparseable /
    /// non-finite ⇒ [`DEFAULT_BOOST`]; a parsed value is clamped to `≥ 0.0` so a
    /// negative boost can only bury a document, never invert ordering.
    #[must_use]
    pub fn parse_boost(metadata: &HashMap<String, String>) -> f32 {
        metadata
            .get(Self::BOOST_KEY)
            .and_then(|raw| raw.trim().parse::<f32>().ok())
            .filter(|b| b.is_finite())
            .map(|b| b.max(0.0))
            .unwrap_or(DEFAULT_BOOST)
    }

    /// Build a [`DocMeta`] from a stored document's metadata.
    #[must_use]
    pub fn from_metadata(metadata: &HashMap<String, String>) -> Self {
        Self {
            document_sets: Self::parse_sets(metadata),
            boost: Self::parse_boost(metadata),
            metadata: metadata.clone(),
        }
    }

    /// Build a [`DocMeta`] from a [`Document`] (convenience over
    /// [`from_metadata`](Self::from_metadata)).
    #[must_use]
    pub fn from_document(doc: &Document) -> Self {
        Self::from_metadata(&doc.metadata)
    }

    /// Whether this document belongs to the named set.
    #[must_use]
    pub fn in_set(&self, set: &str) -> bool {
        self.document_sets.iter().any(|s| s == set)
    }
}

/// Stamp a document-set membership onto a [`Document`]'s metadata (builder).
///
/// Multi-valued: pass several names to tag the document into all of them (stored
/// as the comma-separated `document_set` value the [`CuratedKnowledgeStore`]
/// parses). This is how a connector / ingest config tags a repo's docs into a
/// named set (e.g. dev-support tags every doc from `acme/app` into set
/// `"acme/app"`).
#[must_use]
pub fn with_document_set<I, S>(doc: Document, sets: I) -> Document
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let joined = sets
        .into_iter()
        .map(Into::into)
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join(",");
    if joined.is_empty() {
        doc
    } else {
        doc.with_metadata(DocMeta::DOCUMENT_SET_KEY, joined)
    }
}

/// Stamp a numeric boost onto a [`Document`]'s metadata (builder). A non-finite
/// or negative value is normalized so the stamp is always a sane, parseable
/// multiplier.
#[must_use]
pub fn with_boost(doc: Document, boost: f32) -> Document {
    let boost = if boost.is_finite() {
        boost.max(0.0)
    } else {
        DEFAULT_BOOST
    };
    doc.with_metadata(DocMeta::BOOST_KEY, format!("{boost}"))
}

/// A query-time retrieval filter: scope results to named document sets and/or
/// require metadata equalities.
///
/// - `document_sets: None` ⇒ **no set scoping** (every document is eligible —
///   the current/default behavior). `Some([])` ⇒ scope to *no* set (matches
///   nothing); `Some(["alpha"])` ⇒ only documents in set `"alpha"`; a doc in
///   **any** of the listed sets matches (union).
/// - `metadata_eq` ⇒ every `(key, value)` must be present and equal in the
///   document's stamped metadata (logical AND across the map). Empty ⇒ no
///   metadata constraint.
///
/// An all-default `RetrievalFilter` ([`RetrievalFilter::none`]) matches every
/// document — the no-op that preserves current retrieval behavior.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalFilter {
    /// Scope to documents in any of these sets. `None` ⇒ unscoped (all docs).
    #[serde(default)]
    pub document_sets: Option<Vec<String>>,
    /// Require these `key == value` metadata equalities (all must hold).
    #[serde(default)]
    pub metadata_eq: HashMap<String, String>,
}

impl RetrievalFilter {
    /// The no-op filter: no set scoping, no metadata constraint — matches every
    /// document (preserves default retrieval behavior).
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Scope retrieval to the given document sets (a doc in any of them matches).
    #[must_use]
    pub fn in_sets<I, S>(sets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            document_sets: Some(sets.into_iter().map(Into::into).collect()),
            metadata_eq: HashMap::new(),
        }
    }

    /// Add a required metadata equality (builder).
    #[must_use]
    pub fn with_metadata_eq(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata_eq.insert(key.into(), value.into());
        self
    }

    /// Whether this filter imposes no constraint at all (so retrieval is
    /// unchanged). Used to short-circuit the over-fetch when there's nothing to
    /// filter on.
    #[must_use]
    pub fn is_unconstrained(&self) -> bool {
        self.document_sets.is_none() && self.metadata_eq.is_empty()
    }

    /// Whether a document with the given [`DocMeta`] passes this filter.
    ///
    /// True when (a) `document_sets` is `None` *or* the doc is in at least one of
    /// the listed sets, **and** (b) every `metadata_eq` entry is present and
    /// equal in the doc's metadata.
    #[must_use]
    pub fn matches(&self, meta: &DocMeta) -> bool {
        if let Some(sets) = &self.document_sets {
            if !sets.iter().any(|s| meta.in_set(s)) {
                return false;
            }
        }
        self.metadata_eq
            .iter()
            .all(|(k, v)| meta.metadata.get(k).is_some_and(|mv| mv == v))
    }
}

/// Side table mapping a stored `document_id` to its [`DocMeta`]. Shared (`Arc`)
/// between the ingest handle that populates it and every per-request reader.
type MetaTable = Arc<RwLock<HashMap<String, DocMeta>>>;

/// Side table mapping a stored `document_id` to its [`DocAcl`] (same role as in
/// [`access_control`](crate::access_control)) so a [`CuratedKnowledgeStore`]
/// enforces ACL ∧ curation in one read pass.
type AclTable = Arc<RwLock<HashMap<String, DocAcl>>>;

/// A curation-aware knowledge store: wraps any inner
/// [`KnowledgeBase`](smooth_operator_core::KnowledgeBase), records each
/// document's [`DocMeta`] (set membership / boost / metadata) **and** its
/// [`DocAcl`] at ingest, and mints readers that apply a [`RetrievalFilter`] and
/// [`AccessContext`] together at read time.
///
/// Like [`AclKnowledgeStore`](crate::access_control::AclKnowledgeStore), it does
/// not itself implement `KnowledgeBase` for reading (reads must be bound to a
/// filter + requester). Instead:
/// - [`ingest_handle`](Self::ingest_handle) returns an `Arc<dyn KnowledgeBase>`
///   that records curation metadata + ACL as it ingests;
/// - [`reader`](Self::reader) mints a filtering/boosting `Arc<dyn KnowledgeBase>`
///   bound to a [`RetrievalFilter`] + [`AccessContext`].
#[derive(Clone)]
pub struct CuratedKnowledgeStore {
    inner: Arc<dyn KnowledgeBase>,
    meta: MetaTable,
    acls: AclTable,
}

impl CuratedKnowledgeStore {
    /// Wrap an inner knowledge base. The store starts with empty side tables;
    /// every document ingested through [`ingest_handle`](Self::ingest_handle) has
    /// its [`DocMeta`] (and [`DocAcl`], if stamped) recorded.
    #[must_use]
    pub fn new(inner: Arc<dyn KnowledgeBase>) -> Self {
        Self {
            inner,
            meta: Arc::new(RwLock::new(HashMap::new())),
            acls: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// An ingest-side handle: a [`KnowledgeBase`] whose `ingest` records the
    /// document's [`DocMeta`] (always) and [`DocAcl`] (if stamped) in the shared
    /// side tables, then forwards to the inner backend. Its `query` is the
    /// **unfiltered** inner query — production reads use [`reader`](Self::reader).
    #[must_use]
    pub fn ingest_handle(&self) -> Arc<dyn KnowledgeBase> {
        Arc::new(CuratedIngestHandle {
            inner: Arc::clone(&self.inner),
            meta: Arc::clone(&self.meta),
            acls: Arc::clone(&self.acls),
        })
    }

    /// A read-side handle bound to a [`RetrievalFilter`] + [`AccessContext`]: a
    /// [`KnowledgeBase`] whose `query` over-fetches from the inner backend, drops
    /// every result the requester is not entitled to (ACL) or that does not match
    /// the filter (sets/metadata), multiplies each survivor's score by its boost,
    /// re-sorts, and truncates to the requested limit.
    ///
    /// Pass [`RetrievalFilter::none`] + [`AccessContext::anonymous`] for the
    /// unscoped, ACL-default path (boost still applies — a boosted doc still
    /// re-ranks).
    #[must_use]
    pub fn reader(&self, filter: RetrievalFilter, access: AccessContext) -> Arc<dyn KnowledgeBase> {
        Arc::new(CuratedReader {
            inner: Arc::clone(&self.inner),
            meta: Arc::clone(&self.meta),
            acls: Arc::clone(&self.acls),
            filter,
            access,
        })
    }

    /// Record `document_id → meta` directly (without ingesting a document) — for
    /// callers that store documents through some other path but still want the
    /// curation filter/boost applied at read.
    ///
    /// # Errors
    /// Returns an error if the metadata table lock is poisoned.
    pub fn record_meta(&self, document_id: impl Into<String>, meta: DocMeta) -> anyhow::Result<()> {
        let mut table = self
            .meta
            .write()
            .map_err(|e| anyhow::anyhow!("curation meta table lock poisoned: {e}"))?;
        table.insert(document_id.into(), meta);
        Ok(())
    }
}

/// Records curation metadata + ACL at ingest, forwarding documents to the inner
/// backend.
struct CuratedIngestHandle {
    inner: Arc<dyn KnowledgeBase>,
    meta: MetaTable,
    acls: AclTable,
}

/// Shared ingest bookkeeping: record a document's [`DocMeta`] (always) and
/// [`DocAcl`] (when stamped) into the side tables before forwarding it.
fn record_ingest_metadata(meta: &MetaTable, acls: &AclTable, doc: &Document) -> anyhow::Result<()> {
    {
        let mut table = meta
            .write()
            .map_err(|e| anyhow::anyhow!("curation meta table lock poisoned: {e}"))?;
        table.insert(doc.id.clone(), DocMeta::from_document(doc));
    }
    if let Some(acl) = DocAcl::from_metadata(&doc.metadata) {
        let mut table = acls
            .write()
            .map_err(|e| anyhow::anyhow!("acl table lock poisoned: {e}"))?;
        table.insert(doc.id.clone(), acl);
    }
    Ok(())
}

impl KnowledgeBase for CuratedIngestHandle {
    fn ingest(&self, doc: Document) -> anyhow::Result<()> {
        record_ingest_metadata(&self.meta, &self.acls, &doc)?;
        self.inner.ingest(doc)
    }

    fn query(&self, query: &str, limit: usize) -> anyhow::Result<Vec<KnowledgeResult>> {
        self.inner.query(query, limit)
    }
}

/// Filters + boosts query results by a bound [`RetrievalFilter`] +
/// [`AccessContext`].
struct CuratedReader {
    inner: Arc<dyn KnowledgeBase>,
    meta: MetaTable,
    acls: AclTable,
    filter: RetrievalFilter,
    access: AccessContext,
}

impl KnowledgeBase for CuratedReader {
    fn ingest(&self, doc: Document) -> anyhow::Result<()> {
        // A reader can still ingest (recording metadata + ACL), so the same
        // handle is usable end to end in tests — production ingest uses
        // ingest_handle.
        record_ingest_metadata(&self.meta, &self.acls, &doc)?;
        self.inner.ingest(doc)
    }

    fn query(&self, query: &str, limit: usize) -> anyhow::Result<Vec<KnowledgeResult>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        // Over-fetch so the post-filter, post-boost top-K is full whenever
        // enough matching documents exist.
        let candidate_n = limit.saturating_mul(OVERFETCH_FACTOR).max(OVERFETCH_FLOOR);
        let candidates = self.inner.query(query, candidate_n)?;

        let meta_table = self
            .meta
            .read()
            .map_err(|e| anyhow::anyhow!("curation meta table lock poisoned: {e}"))?;
        let acl_table = self
            .acls
            .read()
            .map_err(|e| anyhow::anyhow!("acl table lock poisoned: {e}"))?;

        let mut kept: Vec<KnowledgeResult> = Vec::with_capacity(candidates.len());
        for mut result in candidates {
            // ACL first: a curation filter must never widen what a requester can
            // see. No recorded ACL ⇒ org-public (backward-compatible default).
            let acl_ok = match acl_table.get(&result.document_id) {
                Some(acl) => self.access.can_access(acl),
                None => true,
            };
            if !acl_ok {
                continue;
            }

            // Then the curation filter (sets + metadata). A document with no
            // recorded DocMeta is treated as an empty DocMeta: it belongs to no
            // set (so a set-scoped query skips it) and has the default boost.
            let doc_meta = meta_table.get(&result.document_id).cloned();
            let meta_for_match = doc_meta.clone().unwrap_or_else(|| DocMeta {
                document_sets: Vec::new(),
                boost: DEFAULT_BOOST,
                metadata: HashMap::new(),
            });
            if !self.filter.matches(&meta_for_match) {
                continue;
            }

            // Apply the boost to the score before re-ranking.
            result.score *= meta_for_match.boost;
            kept.push(result);
        }

        // Re-sort by the boosted score (descending), then truncate to K. A
        // stable, total order: NaN-safe via `total_cmp`.
        kept.sort_by(|a, b| b.score.total_cmp(&a.score));
        kept.truncate(limit);
        Ok(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smooth_operator_core::DocumentType;

    fn doc(id: &str, content: &str) -> Document {
        let mut d = Document::new(content, "s", DocumentType::Documentation);
        d.id = id.to_string();
        d
    }

    // ---- DocMeta::parse_sets --------------------------------------------

    #[test]
    fn parse_sets_single_and_multi() {
        let d = with_document_set(doc("a", "x"), ["alpha"]);
        assert_eq!(DocMeta::parse_sets(&d.metadata), vec!["alpha".to_string()]);

        let d = with_document_set(doc("b", "x"), ["alpha", "beta"]);
        assert_eq!(
            DocMeta::parse_sets(&d.metadata),
            vec!["alpha".to_string(), "beta".to_string()]
        );
    }

    #[test]
    fn parse_sets_trims_and_drops_empties() {
        let d = doc("c", "x").with_metadata(DocMeta::DOCUMENT_SET_KEY, " alpha , , beta ,");
        assert_eq!(
            DocMeta::parse_sets(&d.metadata),
            vec!["alpha".to_string(), "beta".to_string()]
        );
    }

    #[test]
    fn parse_sets_absent_is_empty() {
        let d = doc("d", "x");
        assert!(DocMeta::parse_sets(&d.metadata).is_empty());
    }

    // ---- DocMeta::parse_boost (boost math: default + malformed → 1.0) ----

    #[test]
    fn parse_boost_default_when_absent() {
        let d = doc("e", "x");
        assert_eq!(DocMeta::parse_boost(&d.metadata), DEFAULT_BOOST);
    }

    #[test]
    fn parse_boost_parses_valid() {
        let d = with_boost(doc("f", "x"), 3.0);
        assert!((DocMeta::parse_boost(&d.metadata) - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_boost_malformed_falls_back_to_default() {
        for bad in ["abc", "", "  ", "NaN", "inf", "1.2.3"] {
            let d = doc("g", "x").with_metadata(DocMeta::BOOST_KEY, bad);
            assert_eq!(
                DocMeta::parse_boost(&d.metadata),
                DEFAULT_BOOST,
                "malformed boost {bad:?} must fall back to default"
            );
        }
    }

    #[test]
    fn parse_boost_negative_is_clamped_to_zero() {
        let d = doc("h", "x").with_metadata(DocMeta::BOOST_KEY, "-2.0");
        assert_eq!(DocMeta::parse_boost(&d.metadata), 0.0);
    }

    #[test]
    fn with_boost_normalizes_non_finite() {
        // A non-finite boost passed to the builder is normalized to the default.
        let d = with_boost(doc("i", "x"), f32::NAN);
        assert_eq!(DocMeta::parse_boost(&d.metadata), DEFAULT_BOOST);
        let d = with_boost(doc("j", "x"), f32::INFINITY);
        assert_eq!(DocMeta::parse_boost(&d.metadata), DEFAULT_BOOST);
    }

    // ---- RetrievalFilter::matches (the filter predicate) ----------------

    fn meta(sets: &[&str], boost: f32, kv: &[(&str, &str)]) -> DocMeta {
        let mut metadata = HashMap::new();
        for (k, v) in kv {
            metadata.insert((*k).to_string(), (*v).to_string());
        }
        DocMeta {
            document_sets: sets.iter().map(ToString::to_string).collect(),
            boost,
            metadata,
        }
    }

    #[test]
    fn unconstrained_filter_matches_everything() {
        let f = RetrievalFilter::none();
        assert!(f.is_unconstrained());
        assert!(f.matches(&meta(&[], 1.0, &[])));
        assert!(f.matches(&meta(&["alpha"], 1.0, &[("kind", "code")])));
    }

    #[test]
    fn set_scope_matches_only_members() {
        let f = RetrievalFilter::in_sets(["alpha"]);
        assert!(!f.is_unconstrained());
        assert!(f.matches(&meta(&["alpha"], 1.0, &[])));
        assert!(f.matches(&meta(&["alpha", "beta"], 1.0, &[]))); // multi-set member
        assert!(!f.matches(&meta(&["beta"], 1.0, &[])));
        assert!(!f.matches(&meta(&[], 1.0, &[]))); // in no set
    }

    #[test]
    fn set_scope_union_across_listed_sets() {
        let f = RetrievalFilter::in_sets(["alpha", "gamma"]);
        assert!(f.matches(&meta(&["gamma"], 1.0, &[])));
        assert!(f.matches(&meta(&["alpha"], 1.0, &[])));
        assert!(!f.matches(&meta(&["beta"], 1.0, &[])));
    }

    #[test]
    fn empty_set_list_matches_nothing() {
        let f = RetrievalFilter {
            document_sets: Some(vec![]),
            metadata_eq: HashMap::new(),
        };
        assert!(!f.matches(&meta(&["alpha"], 1.0, &[])));
        assert!(!f.matches(&meta(&[], 1.0, &[])));
    }

    #[test]
    fn metadata_eq_requires_all_equalities() {
        let f = RetrievalFilter::none()
            .with_metadata_eq("kind", "prose")
            .with_metadata_eq("lang", "en");
        assert!(f.matches(&meta(&[], 1.0, &[("kind", "prose"), ("lang", "en")])));
        // Missing one key.
        assert!(!f.matches(&meta(&[], 1.0, &[("kind", "prose")])));
        // Wrong value.
        assert!(!f.matches(&meta(&[], 1.0, &[("kind", "code"), ("lang", "en")])));
    }

    #[test]
    fn set_and_metadata_compose_with_and() {
        let f = RetrievalFilter::in_sets(["alpha"]).with_metadata_eq("kind", "prose");
        assert!(f.matches(&meta(&["alpha"], 1.0, &[("kind", "prose")])));
        assert!(!f.matches(&meta(&["alpha"], 1.0, &[("kind", "code")]))); // set ok, meta no
        assert!(!f.matches(&meta(&["beta"], 1.0, &[("kind", "prose")]))); // meta ok, set no
    }

    // ---- RetrievalFilter round-trips through serde ----------------------

    #[test]
    fn filter_round_trips_through_json() {
        let f = RetrievalFilter::in_sets(["alpha", "beta"]).with_metadata_eq("kind", "prose");
        let json = serde_json::to_string(&f).expect("serialize");
        let parsed: RetrievalFilter = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, f);
    }

    // ---- store-level: ingest records DocMeta, reader applies it ---------

    fn curated_store() -> CuratedKnowledgeStore {
        CuratedKnowledgeStore::new(Arc::new(smooth_operator_core::InMemoryKnowledge::new()))
    }

    #[test]
    fn reader_with_no_filter_returns_all_with_boost_applied() {
        let store = curated_store();
        let h = store.ingest_handle();
        h.ingest(with_document_set(
            doc("a", "clearance alpha fact"),
            ["alpha"],
        ))
        .unwrap();
        h.ingest(doc("plain", "clearance plain fact")).unwrap();

        // Unconstrained filter: both come back.
        let r = store.reader(RetrievalFilter::none(), AccessContext::anonymous());
        let ids: Vec<String> = r
            .query("clearance", 10)
            .unwrap()
            .into_iter()
            .map(|x| x.document_id)
            .collect();
        assert!(ids.contains(&"a".to_string()));
        assert!(ids.contains(&"plain".to_string()));
    }

    #[test]
    fn malformed_boost_metadata_yields_default_boost_at_read() {
        let store = curated_store();
        let h = store.ingest_handle();
        // A doc whose boost stamp is garbage must not vanish or explode — it
        // ranks with the default 1.0 boost.
        h.ingest(doc("bad", "clearance fact").with_metadata(DocMeta::BOOST_KEY, "not-a-number"))
            .unwrap();
        let r = store.reader(RetrievalFilter::none(), AccessContext::anonymous());
        let hits = r.query("clearance", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document_id, "bad");
    }
}
