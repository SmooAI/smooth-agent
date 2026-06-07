//! Document → chunks (Onyx-gap G2).
//!
//! The engine's `KnowledgeBase::ingest` chunks internally, but that's tuned for
//! whole pre-formed documents. The ingestion pipeline owns its own chunker so
//! the chunk shape (size cap, overlap, stable ids, propagated metadata) is
//! explicit and tested independently of any storage backend.
//!
//! ## Strategy
//!
//! 1. Split content into paragraphs on blank lines (`\n\n`).
//! 2. Greedily pack paragraphs into a chunk up to [`Chunker::max_chars`].
//! 3. A single paragraph larger than the cap is hard-split on word boundaries.
//! 4. Successive chunks overlap by [`Chunker::overlap_chars`] of trailing text
//!    (carried as whole words) so a fact spanning a boundary stays retrievable.
//!
//! Each [`Chunk`] gets a **stable id** — `"{doc_id}#{index}"` — and inherits the
//! source document's title/metadata/acl, so retrieval can attribute and (later)
//! access-control every chunk.

use std::collections::HashMap;

use crate::connector::RawDocument;

/// Default maximum characters per chunk. Matches the engine's in-memory
/// `MAX_CHUNK_CHARS` so chunk granularity is consistent end to end.
pub const DEFAULT_MAX_CHARS: usize = 500;

/// Default overlap (characters of trailing text repeated into the next chunk).
pub const DEFAULT_OVERLAP_CHARS: usize = 64;

/// One chunk produced from a [`RawDocument`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Stable id: `"{doc_id}#{index}"`.
    pub id: String,
    /// The originating document's id.
    pub document_id: String,
    /// 0-based position within the document.
    pub index: usize,
    /// The chunk text.
    pub text: String,
    /// Title/metadata/acl propagated from the source document. `title` (if any)
    /// is also stored under the `"title"` metadata key for retrieval display.
    pub metadata: HashMap<String, String>,
    /// Access-control labels propagated from the source document (G3).
    pub acl: Option<Vec<String>>,
}

/// Splits documents into overlapping, size-capped chunks.
#[derive(Debug, Clone)]
pub struct Chunker {
    max_chars: usize,
    overlap_chars: usize,
}

impl Chunker {
    /// Build with explicit `max_chars` and `overlap_chars`.
    ///
    /// `overlap_chars` is clamped below `max_chars` so a chunk always makes
    /// forward progress (an overlap ≥ size would loop forever).
    #[must_use]
    pub fn new(max_chars: usize, overlap_chars: usize) -> Self {
        let max_chars = max_chars.max(1);
        Self {
            max_chars,
            overlap_chars: overlap_chars.min(max_chars.saturating_sub(1)),
        }
    }

    /// The configured max characters per chunk.
    #[must_use]
    pub fn max_chars(&self) -> usize {
        self.max_chars
    }

    /// The configured overlap in characters.
    #[must_use]
    pub fn overlap_chars(&self) -> usize {
        self.overlap_chars
    }

    /// Chunk a [`RawDocument`], returning its ordered [`Chunk`]s.
    ///
    /// An empty / whitespace-only document yields no chunks.
    #[must_use]
    pub fn chunk(&self, doc: &RawDocument) -> Vec<Chunk> {
        let texts = self.split_text(&doc.content);

        // Build the per-chunk metadata once (title folded in), clone per chunk.
        let mut base_meta = doc.metadata.clone();
        if let Some(title) = &doc.title {
            base_meta
                .entry("title".to_string())
                .or_insert_with(|| title.clone());
        }
        base_meta
            .entry("source".to_string())
            .or_insert_with(|| doc.source.clone());

        texts
            .into_iter()
            .enumerate()
            .map(|(index, text)| Chunk {
                id: format!("{}#{index}", doc.id),
                document_id: doc.id.clone(),
                index,
                text,
                metadata: base_meta.clone(),
                acl: doc.acl.clone(),
            })
            .collect()
    }

    /// Split raw content into chunk-sized texts (no metadata; pure string work).
    fn split_text(&self, content: &str) -> Vec<String> {
        // 1. Paragraph units (blank-line separated), oversized ones hard-split.
        let mut units: Vec<String> = Vec::new();
        for para in content.split("\n\n") {
            let trimmed = para.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.chars().count() <= self.max_chars {
                units.push(trimmed.to_string());
            } else {
                units.extend(self.hard_split_words(trimmed));
            }
        }

        // 2. Greedily pack units, then 3. add trailing-word overlap.
        let mut chunks: Vec<String> = Vec::new();
        let mut current = String::new();
        for unit in units {
            if current.is_empty() {
                current = unit;
            } else if current.chars().count() + 2 + unit.chars().count() <= self.max_chars {
                current.push_str("\n\n");
                current.push_str(&unit);
            } else {
                chunks.push(std::mem::take(&mut current));
                current = unit;
            }
        }
        if !current.is_empty() {
            chunks.push(current);
        }

        self.apply_overlap(chunks)
    }

    /// Hard-split a single oversized paragraph at word boundaries.
    fn hard_split_words(&self, para: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        for word in para.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.chars().count() + 1 + word.chars().count() > self.max_chars {
                out.push(std::mem::take(&mut current));
                current.push_str(word);
            } else {
                current.push(' ');
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
        out
    }

    /// Prepend the trailing `overlap_chars` (rounded to whole words) of each
    /// chunk onto the next, so a boundary-spanning fact appears in both.
    fn apply_overlap(&self, chunks: Vec<String>) -> Vec<String> {
        if self.overlap_chars == 0 || chunks.len() < 2 {
            return chunks;
        }
        let mut out = Vec::with_capacity(chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            if i == 0 {
                out.push(chunk.clone());
                continue;
            }
            let tail = self.trailing_words(&chunks[i - 1]);
            if tail.is_empty() {
                out.push(chunk.clone());
            } else {
                out.push(format!("{tail} {chunk}"));
            }
        }
        out
    }

    /// The last whole words of `s` totaling at most `overlap_chars` characters.
    fn trailing_words(&self, s: &str) -> String {
        let words: Vec<&str> = s.split_whitespace().collect();
        let mut take = 0usize;
        let mut len = 0usize;
        for word in words.iter().rev() {
            let add = word.chars().count() + usize::from(take > 0);
            if len + add > self.overlap_chars {
                break;
            }
            len += add;
            take += 1;
        }
        if take == 0 {
            return String::new();
        }
        words[words.len() - take..].join(" ")
    }
}

impl Default for Chunker {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CHARS, DEFAULT_OVERLAP_CHARS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_doc_is_a_single_chunk() {
        let doc = RawDocument::new("d", "test", "just a short note");
        let chunks = Chunker::default().chunk(&doc);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "just a short note");
        assert_eq!(chunks[0].id, "d#0");
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[0].document_id, "d");
    }

    #[test]
    fn empty_doc_yields_no_chunks() {
        let doc = RawDocument::new("d", "test", "   \n\n   ");
        assert!(Chunker::default().chunk(&doc).is_empty());
    }

    #[test]
    fn paragraphs_pack_then_split_at_cap() {
        // max 20 chars, no overlap → each ~15-char paragraph is its own chunk
        // because two won't fit (15 + 2 + 15 > 20).
        let chunker = Chunker::new(20, 0);
        let doc = RawDocument::new(
            "d",
            "test",
            "paragraph one!!\n\nparagraph two!!\n\nparagraph thr!!",
        );
        let chunks = chunker.chunk(&doc);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].text, "paragraph one!!");
        assert_eq!(chunks[1].text, "paragraph two!!");
        assert_eq!(chunks[2].index, 2);
    }

    #[test]
    fn small_paragraphs_pack_into_one_chunk() {
        let chunker = Chunker::new(100, 0);
        let doc = RawDocument::new("d", "test", "aaa\n\nbbb\n\nccc");
        let chunks = chunker.chunk(&doc);
        assert_eq!(chunks.len(), 1, "small paragraphs should pack together");
        assert!(chunks[0].text.contains("aaa"));
        assert!(chunks[0].text.contains("ccc"));
    }

    #[test]
    fn oversized_paragraph_hard_splits_on_words() {
        let chunker = Chunker::new(10, 0);
        // One paragraph, no blank lines, longer than the cap.
        let doc = RawDocument::new("d", "test", "alpha beta gamma delta epsilon");
        let chunks = chunker.chunk(&doc);
        assert!(chunks.len() > 1, "oversized paragraph must split");
        for c in &chunks {
            assert!(
                c.text.chars().count() <= 10,
                "chunk exceeds cap: {:?}",
                c.text
            );
        }
    }

    #[test]
    fn overlap_carries_trailing_words_into_next_chunk() {
        let chunker = Chunker::new(20, 8);
        let doc = RawDocument::new(
            "d",
            "test",
            "first chunk text\n\nsecond chunk text\n\nthird chunk text",
        );
        let chunks = chunker.chunk(&doc);
        assert!(chunks.len() >= 2);
        // The second chunk should begin with a trailing word of the first.
        let prev_last = chunks[0]
            .text
            .split_whitespace()
            .last()
            .unwrap()
            .to_string();
        assert!(
            chunks[1].text.starts_with(&prev_last),
            "expected overlap word {prev_last:?} at start of {:?}",
            chunks[1].text
        );
    }

    #[test]
    fn overlap_is_clamped_below_max_so_it_terminates() {
        // overlap >= max would loop; constructor clamps it.
        let chunker = Chunker::new(10, 999);
        assert!(chunker.overlap_chars() < chunker.max_chars());
        let doc = RawDocument::new("d", "test", "alpha beta gamma delta epsilon zeta");
        let chunks = chunker.chunk(&doc); // must terminate
        assert!(!chunks.is_empty());
    }

    #[test]
    fn metadata_and_title_propagate_to_every_chunk() {
        let chunker = Chunker::new(15, 0);
        let doc = RawDocument::new("d", "wiki", "alpha words here\n\nbeta words here")
            .with_title("My Title")
            .with_metadata("category", "facts")
            .with_acl(vec!["group-a".to_string()]);
        let chunks = chunker.chunk(&doc);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert_eq!(
                c.metadata.get("title").map(String::as_str),
                Some("My Title")
            );
            assert_eq!(
                c.metadata.get("category").map(String::as_str),
                Some("facts")
            );
            assert_eq!(c.metadata.get("source").map(String::as_str), Some("wiki"));
            assert_eq!(c.acl.as_deref(), Some(&["group-a".to_string()][..]));
        }
    }

    #[test]
    fn chunk_ids_are_stable_and_indexed() {
        let chunker = Chunker::new(15, 0);
        let doc = RawDocument::new("doc-42", "test", "alpha words!!\n\nbeta words!!");
        let chunks = chunker.chunk(&doc);
        assert_eq!(chunks[0].id, "doc-42#0");
        assert_eq!(chunks[1].id, "doc-42#1");
        // Re-chunking the same input yields the same ids (stable).
        let again = chunker.chunk(&doc);
        assert_eq!(chunks, again);
    }
}
