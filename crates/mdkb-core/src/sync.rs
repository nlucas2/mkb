//! Keeping the index in step with the vault on disk.
//!
//! [`SyncEngine`] owns the in-memory [`Vault`] and an [`Index`], and reconciles them with a
//! directory of Markdown files. It is the piece a file watcher (in the daemon) drives: on a
//! file change it re-ingests one page; on startup it reconciles the whole tree, skipping
//! files whose content hash is unchanged.
//!
//! Markdown is always the source of truth; the index is rebuilt from it. Ids are assigned
//! eagerly: ingesting a page writes invisible markers back to disk so every block has a
//! stable identity (this is what makes incremental re-indexing deterministic).

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::document::Document;
use crate::embed::Embedder;
use crate::id::NativeIdCodec;
use crate::index::{page_links, Index, IndexError, SearchHit, SearchQuery};
use crate::vault::{markdown_files, Vault};

/// Result of a reconcile pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Pages added or updated.
    pub changed: Vec<String>,
    /// Pages removed.
    pub removed: Vec<String>,
    /// Cloud-sync conflict files detected (surfaced, not indexed).
    pub conflicts: Vec<String>,
}

impl SyncReport {
    /// Whether anything changed (conflicts are informational, not a change).
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty()
    }
}

/// Owns a vault + index and keeps them synced with a directory.
pub struct SyncEngine<I: Index> {
    root: PathBuf,
    vault: Vault,
    index: I,
    hashes: HashMap<String, u64>,
    assign_ids: bool,
    embedder: Option<Box<dyn Embedder>>,
    conflicts: Vec<String>,
}

fn hash_bytes(b: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    b.hash(&mut h);
    h.finish()
}

fn io_err(e: impl std::fmt::Display) -> IndexError {
    IndexError::new(e)
}

impl<I: Index> SyncEngine<I> {
    /// Create an engine rooted at `root` with the given index.
    pub fn new(root: impl Into<PathBuf>, index: I) -> Self {
        SyncEngine {
            root: root.into(),
            vault: Vault::new(),
            index,
            hashes: HashMap::new(),
            assign_ids: true,
            embedder: None,
            conflicts: Vec::new(),
        }
    }

    /// Disable eager id assignment (the engine will not modify files on ingest).
    pub fn without_id_assignment(mut self) -> Self {
        self.assign_ids = false;
        self
    }

    /// Attach an embedder so blocks get embeddings on ingest and queries become semantic.
    pub fn with_embedder(mut self, embedder: Box<dyn Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// The vault root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Borrow the vault (read-only).
    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    /// Borrow the index (read-only).
    pub fn index(&self) -> &I {
        &self.index
    }

    /// Mutably borrow the index.
    pub fn index_mut(&mut self) -> &mut I {
        &mut self.index
    }

    /// Reconcile the entire tree: ingest new/changed pages, drop deleted ones. Files whose
    /// on-disk content hash is unchanged since the last pass are skipped.
    pub fn reconcile(&mut self) -> Result<SyncReport, IndexError> {
        let files = markdown_files(&self.root).map_err(io_err)?;
        let mut report = SyncReport::default();
        let mut seen = HashSet::new();
        let mut conflicts = Vec::new();
        for (rel, abs) in files {
            // Cloud-sync conflict copies are surfaced, never indexed (would duplicate /
            // stale-pollute search). The human resolves them in plain text.
            if crate::conflict::is_conflict_path(&rel) {
                conflicts.push(rel.clone());
                if self.hashes.contains_key(&rel) {
                    // It was previously a normal file that got mangled; drop its old index.
                    self.drop_page(&rel)?;
                }
                continue;
            }
            seen.insert(rel.clone());
            let bytes = std::fs::read(&abs).map_err(io_err)?;
            if self.hashes.get(&rel) == Some(&hash_bytes(&bytes)) {
                continue;
            }
            // Markdown is the source of truth: never lossily rewrite a file we can't decode.
            // Non-UTF-8 files are skipped (and dropped from the index if they were valid
            // before) rather than mangled with U+FFFD replacements.
            let source = match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(_) => {
                    eprintln!("mdkb: skipping non-UTF-8 file {rel}");
                    seen.remove(&rel);
                    if self.hashes.contains_key(&rel) {
                        self.drop_page(&rel)?;
                    }
                    continue;
                }
            };
            self.ingest_source(&rel, source)?;
            report.changed.push(rel);
        }
        let removed: Vec<String> = self
            .hashes
            .keys()
            .filter(|k| !seen.contains(*k))
            .cloned()
            .collect();
        for rel in removed {
            self.drop_page(&rel)?;
            report.removed.push(rel);
        }
        conflicts.sort();
        self.conflicts = conflicts;
        report.changed.sort();
        report.removed.sort();
        report.conflicts = self.conflicts.clone();
        Ok(report)
    }

    /// Cloud-sync conflict files detected at the last reconcile (surfaced, not indexed).
    pub fn conflicts(&self) -> &[String] {
        &self.conflicts
    }

    /// Rebuild the entire index from scratch off the current vault files. Clears the index,
    /// forgets cached hashes, and re-ingests everything. Use after corruption or a model
    /// change. The Markdown is untouched (it is the source of truth).
    pub fn rebuild(&mut self) -> Result<SyncReport, IndexError> {
        self.index.clear()?;
        self.vault = Vault::new();
        self.hashes.clear();
        self.conflicts.clear();
        self.reconcile()
    }

    /// Ingest a single page from disk (reading `root/rel`).
    pub fn ingest_path(&mut self, rel: &str) -> Result<(), IndexError> {
        let abs = self.root.join(rel);
        let source = std::fs::read_to_string(&abs).map_err(io_err)?;
        self.ingest_source(rel, source)
    }

    /// Save a page: write `source` to disk (assigning ids first) and index it. This is the
    /// canonical "persist a page" operation used by write APIs.
    pub fn save_page(&mut self, rel: &str, source: impl Into<String>) -> Result<(), IndexError> {
        self.ingest_source(rel, source.into())
    }

    /// Remove a page from vault + index *and* delete its file from disk.
    pub fn delete_page(&mut self, rel: &str) -> Result<(), IndexError> {
        let rel = crate::vault::safe_relative_path(rel).map_err(IndexError::new)?;
        let abs = self.root.join(&rel);
        if abs.exists() {
            std::fs::remove_file(&abs).map_err(io_err)?;
        }
        self.drop_page(&rel)
    }

    /// Drop a page from vault + index without touching disk (e.g. a file was deleted
    /// externally).
    pub fn drop_page(&mut self, rel: &str) -> Result<(), IndexError> {
        self.vault.remove(rel);
        self.index.remove_page(rel)?;
        self.hashes.remove(rel);
        Ok(())
    }

    /// Core ingest: confine the path to the vault, optionally assign ids (writing the file),
    /// update the vault, recompute links, reindex the page, and record the on-disk content
    /// hash. Confinement here means no write path — internal or via a write API — can escape
    /// the vault root through `..` or an absolute path.
    fn ingest_source(&mut self, rel: &str, source: String) -> Result<(), IndexError> {
        let rel = crate::vault::safe_relative_path(rel).map_err(IndexError::new)?;
        let rel = rel.as_str();
        let final_source = if self.assign_ids {
            let doc = Document::parse(&source);
            match doc.with_assigned_ids(&NativeIdCodec) {
                Some(assigned) => {
                    let abs = self.root.join(rel);
                    if let Some(parent) = abs.parent() {
                        std::fs::create_dir_all(parent).map_err(io_err)?;
                    }
                    std::fs::write(&abs, &assigned).map_err(io_err)?;
                    assigned
                }
                None => source,
            }
        } else {
            source
        };

        self.vault.insert(rel, final_source.clone());
        let page = self
            .vault
            .page(rel)
            .ok_or_else(|| IndexError::new(format!("page vanished after insert: {rel}")))?
            .clone();
        let links = page_links(&self.vault, &page);
        self.index.reindex_page(&page, &links)?;

        if let Some(embedder) = &self.embedder {
            let texts: Vec<String> = page
                .doc
                .blocks
                .iter()
                .map(|b| b.contextual_text())
                .collect();
            let vectors = embedder.embed(&texts).map_err(IndexError::new)?;
            let model_id = embedder.model_id();
            for (block, vector) in page.doc.blocks.iter().zip(vectors) {
                self.index.set_embedding(&block.id, &model_id, &vector)?;
            }
        }

        self.hashes
            .insert(rel.to_string(), hash_bytes(final_source.as_bytes()));
        Ok(())
    }

    /// Search the index, embedding the query text first when an embedder is attached and the
    /// query does not already carry a vector. This is the entry point a daemon/UI uses so
    /// keyword and semantic search are fused automatically. The query is tagged with the
    /// embedder's model id so only same-model vectors are compared.
    pub fn search(&self, mut query: SearchQuery) -> Result<Vec<SearchHit>, IndexError> {
        if query.vector.is_none() {
            if let (Some(embedder), Some(text)) = (&self.embedder, &query.text) {
                let vector = embedder.embed_one(text).map_err(IndexError::new)?;
                query.vector = Some(vector);
                query.vector_model = Some(embedder.model_id());
            }
        }
        self.index.search(&query)
    }

    /// Update an existing block's text by id, persisting the change to disk and reindexing
    /// (and re-embedding) its page. Returns `false` if the id is unknown.
    pub fn update_block(
        &mut self,
        id: &crate::id::BlockId,
        new_text: &str,
    ) -> Result<bool, IndexError> {
        let path = match self.vault.block(id) {
            Some((p, _)) => p.path.clone(),
            None => return Ok(false),
        };
        let new_source = self
            .vault
            .update_block(id, new_text)
            .ok_or_else(|| IndexError::new("block vanished during update"))?;
        self.save_page(&path, new_source)?;
        Ok(true)
    }

    /// Append a new block of `text` to a page, creating the page if it does not exist.
    /// Returns the id assigned to the new block.
    pub fn append_block(
        &mut self,
        page: &str,
        text: &str,
    ) -> Result<crate::id::BlockId, IndexError> {
        let mut source = self
            .vault
            .page(page)
            .map(|p| p.doc.source.clone())
            .unwrap_or_default();
        if !source.is_empty() {
            if !source.ends_with('\n') {
                source.push('\n');
            }
            source.push('\n');
        }
        source.push_str(text.trim_end());
        source.push('\n');
        self.save_page(page, source)?;
        // The new block is the last one on the page after re-parse.
        let id = self
            .vault
            .page(page)
            .and_then(|p| p.doc.blocks.last())
            .map(|b| b.id.clone())
            .ok_or_else(|| IndexError::new("append produced no block"))?;
        Ok(id)
    }

    /// List vault-relative page paths.
    pub fn page_paths(&self) -> Vec<String> {
        self.vault.pages().iter().map(|p| p.path.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::testing::MemIndex;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn reconcile_ingests_and_assigns_ids() {
        let root = temp_root();
        std::fs::write(root.path().join("a.md"), "# A\n\nbody\n").unwrap();
        let mut engine = SyncEngine::new(root.path(), MemIndex::default());
        let report = engine.reconcile().unwrap();
        assert_eq!(report.changed, vec!["a.md".to_string()]);
        assert_eq!(engine.index().stats().unwrap().blocks, 2);
        // Ids were written back to disk.
        let on_disk = std::fs::read_to_string(root.path().join("a.md")).unwrap();
        assert!(on_disk.contains("<!-- mdkb:"));
    }

    #[test]
    fn reconcile_skips_unchanged_files() {
        let root = temp_root();
        std::fs::write(root.path().join("a.md"), "# A\n\nbody\n").unwrap();
        let mut engine = SyncEngine::new(root.path(), MemIndex::default());
        engine.reconcile().unwrap();
        // Second pass: ids already assigned, content hash stable → nothing changes.
        let report = engine.reconcile().unwrap();
        assert!(report.is_empty(), "unchanged tree should be a no-op");
    }

    #[test]
    fn reconcile_detects_deletions() {
        let root = temp_root();
        let path = root.path().join("a.md");
        std::fs::write(&path, "hello\n").unwrap();
        let mut engine = SyncEngine::new(root.path(), MemIndex::default());
        engine.reconcile().unwrap();
        std::fs::remove_file(&path).unwrap();
        let report = engine.reconcile().unwrap();
        assert_eq!(report.removed, vec!["a.md".to_string()]);
        assert_eq!(engine.index().stats().unwrap().pages, 0);
    }

    #[test]
    fn save_page_writes_and_indexes() {
        let root = temp_root();
        let mut engine = SyncEngine::new(root.path(), MemIndex::default());
        engine.save_page("topic/new.md", "fresh note\n").unwrap();
        assert!(root.path().join("topic/new.md").exists());
        assert_eq!(engine.index().stats().unwrap().blocks, 1);
    }

    #[test]
    fn write_apis_reject_path_traversal() {
        let root = temp_root();
        let mut engine = SyncEngine::new(root.path(), MemIndex::default());
        // Absolute and `..` paths must be refused, and nothing must be written outside root.
        assert!(engine.save_page("../escape.md", "x\n").is_err());
        assert!(engine.save_page("/tmp/mdkb-escape-test.md", "x\n").is_err());
        assert!(engine.delete_page("../../etc/passwd").is_err());
        assert!(!std::path::Path::new("/tmp/mdkb-escape-test.md").exists());
        // The parent of root must be untouched.
        let sibling = root.path().parent().unwrap().join("escape.md");
        assert!(!sibling.exists());
    }

    #[test]
    fn conflict_files_are_surfaced_not_indexed() {
        let root = temp_root();
        std::fs::write(root.path().join("arch.md"), "the real note\n").unwrap();
        std::fs::write(
            root.path().join("arch-DESKTOP-AB12CD.md"),
            "a stale conflict copy\n",
        )
        .unwrap();
        let mut engine = SyncEngine::new(root.path(), MemIndex::default());
        let report = engine.reconcile().unwrap();
        // Only the real file is indexed; the conflict copy is reported separately.
        assert_eq!(report.changed, vec!["arch.md".to_string()]);
        assert_eq!(engine.conflicts(), &["arch-DESKTOP-AB12CD.md".to_string()]);
        assert_eq!(report.conflicts, vec!["arch-DESKTOP-AB12CD.md".to_string()]);
        assert_eq!(engine.index().stats().unwrap().pages, 1);
    }

    #[test]
    fn rebuild_reconstructs_index_from_vault() {
        let root = temp_root();
        std::fs::write(root.path().join("a.md"), "alpha\n\nbeta\n").unwrap();
        let mut engine = SyncEngine::new(root.path(), MemIndex::default());
        engine.reconcile().unwrap();
        let before = engine.index().stats().unwrap().blocks;
        assert!(before >= 2);
        let report = engine.rebuild().unwrap();
        // Everything is re-ingested; counts match.
        assert_eq!(engine.index().stats().unwrap().blocks, before);
        assert!(report.changed.contains(&"a.md".to_string()));
    }
}
