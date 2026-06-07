//! [`FileConnector`] — pull `.txt` / `.md` documents from a file or directory.
//!
//! Points at a path. If the path is a file, it yields one [`RawDocument`]; if
//! it's a directory, it walks it (recursively) and yields one document per
//! text/markdown file. The document `id` is the file's path (stable across
//! runs, so re-ingesting an unchanged tree is idempotent), and the `title`
//! defaults to the file stem.
//!
//! Entirely local — no network — so its tests run on every PR (G9 `unit` tier).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::connector::{Connector, RawDocument, Timestamp};

/// File extensions treated as ingestible text.
const TEXT_EXTENSIONS: &[&str] = &["txt", "md", "markdown", "mdx", "text"];

/// Reads text/markdown files from a path (file or directory) as documents.
pub struct FileConnector {
    root: PathBuf,
    recursive: bool,
}

impl FileConnector {
    /// Build a connector rooted at `path`. Directories are walked recursively.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            root: path.into(),
            recursive: true,
        }
    }

    /// Limit a directory walk to the top level only (builder).
    #[must_use]
    pub fn non_recursive(mut self) -> Self {
        self.recursive = false;
        self
    }

    /// Whether `path`'s extension marks it as ingestible text.
    fn is_text_file(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| TEXT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
            .unwrap_or(false)
    }

    /// Collect ingestible file paths under `root` into `out`.
    fn collect_files(&self, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        let entries = std::fs::read_dir(dir)
            .with_context(|| format!("reading directory {}", dir.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| format!("reading entry in {}", dir.display()))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("statting {}", path.display()))?;
            if file_type.is_dir() {
                if self.recursive {
                    self.collect_files(&path, out)?;
                }
            } else if file_type.is_file() && Self::is_text_file(&path) {
                out.push(path);
            }
        }
        Ok(())
    }

    /// Read one file into a [`RawDocument`] (id = path, title = file stem).
    fn read_doc(path: &Path) -> Result<RawDocument> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading file {}", path.display()))?;
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
            .to_string();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        Ok(RawDocument::new(path.to_string_lossy(), "file", content)
            .with_title(title)
            .with_metadata("path", path.to_string_lossy())
            .with_metadata("extension", ext))
    }
}

#[async_trait]
impl Connector for FileConnector {
    fn name(&self) -> &str {
        "file"
    }

    async fn pull(&self, _since: Option<Timestamp>) -> Result<Vec<RawDocument>> {
        let meta = std::fs::metadata(&self.root)
            .with_context(|| format!("statting {}", self.root.display()))?;

        if meta.is_file() {
            // A direct file path is read even if its extension isn't in the
            // text list — the caller asked for it explicitly.
            return Ok(vec![Self::read_doc(&self.root)?]);
        }

        let mut paths = Vec::new();
        self.collect_files(&self.root, &mut paths)?;
        // Deterministic order so reports/tests are stable.
        paths.sort();
        paths.iter().map(|p| Self::read_doc(p)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(dir: &Path, name: &str, body: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    #[tokio::test]
    async fn directory_yields_one_doc_per_text_file() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "a.md", "# Alpha\n\nfirst file");
        write_file(dir.path(), "b.txt", "second file");
        // A non-text file must be ignored.
        write_file(dir.path(), "ignore.bin", "not text");

        let connector = FileConnector::new(dir.path());
        let docs = connector.pull(None).await.unwrap();
        assert_eq!(docs.len(), 2, "only the two text files");

        let ids: Vec<&str> = docs.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.iter().any(|i| i.ends_with("a.md")));
        assert!(ids.iter().any(|i| i.ends_with("b.txt")));
        assert!(docs.iter().all(|d| d.source == "file"));
        assert!(docs.iter().any(|d| d.content.contains("first file")));
    }

    #[tokio::test]
    async fn single_file_path_yields_one_doc() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "only.md", "just one");
        let connector = FileConnector::new(dir.path().join("only.md"));
        let docs = connector.pull(None).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title.as_deref(), Some("only"));
        assert_eq!(docs[0].content, "just one");
    }

    #[tokio::test]
    async fn recursive_walk_descends_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "top.md", "top");
        write_file(dir.path(), "sub/nested.md", "nested");

        let recursive = FileConnector::new(dir.path()).pull(None).await.unwrap();
        assert_eq!(recursive.len(), 2, "recursive default finds nested file");

        let shallow = FileConnector::new(dir.path())
            .non_recursive()
            .pull(None)
            .await
            .unwrap();
        assert_eq!(shallow.len(), 1, "non-recursive stays at top level");
    }

    #[tokio::test]
    async fn ids_are_stable_across_pulls() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "x.txt", "content");
        let connector = FileConnector::new(dir.path());
        let a = connector.pull(None).await.unwrap();
        let b = connector.pull(None).await.unwrap();
        assert_eq!(a[0].id, b[0].id, "path-based id is stable");
    }
}
