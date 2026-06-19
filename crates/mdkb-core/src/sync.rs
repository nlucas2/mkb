//! Keeping the index in step with the vault on disk (the `blocks/<ulid>.md` files).
//!
//! [`SyncEngine`] owns the in-memory [`Vault`] and an [`Index`], and reconciles them with the
//! `blocks/` directory. The daemon's file watcher drives it: on a change it re-ingests one
//! block file; on startup it reconciles the whole directory, skipping files whose content hash
//! is unchanged. Markdown is always the source of truth; the index is rebuilt from it.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::blockfile::write_block;
use crate::embed::Embedder;
use crate::id::BlockId;
use crate::index::{block_links, BlockRecord, Index, IndexError, SearchHit, SearchQuery};
use crate::vault::{block_rel_path, read_block_files, Vault, BLOCKS_DIR};

/// Result of a reconcile pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Block ids added or updated.
    pub changed: Vec<String>,
    /// Block ids removed.
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

/// Owns a vault + index and keeps them synced with the `blocks/` directory.
pub struct SyncEngine<I: Index> {
    root: PathBuf,
    vault: Vault,
    index: I,
    hashes: HashMap<BlockId, u64>,
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
            embedder: None,
            conflicts: Vec::new(),
        }
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

    /// The `blocks/` directory under the root.
    pub fn blocks_dir(&self) -> PathBuf {
        self.root.join(BLOCKS_DIR)
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

    /// Cloud-sync conflict files detected at the last reconcile.
    pub fn conflicts(&self) -> &[String] {
        &self.conflicts
    }

    /// Reconcile the whole `blocks/` directory: ingest new/changed block files, drop deleted
    /// ones. Files whose content hash is unchanged since the last pass are skipped.
    pub fn reconcile(&mut self) -> Result<SyncReport, IndexError> {
        let files = read_block_files(&self.root).map_err(io_err)?;
        let mut report = SyncReport::default();
        let mut seen = HashSet::new();
        let mut conflicts = Vec::new();

        // Surface cloud-sync conflict copies (e.g. "foo (conflicted copy).md") without
        // indexing them — they would duplicate / stale-pollute search.
        for name in conflict_file_names(&self.root) {
            conflicts.push(name);
        }

        for (id, _abs, source) in files {
            seen.insert(id.clone());
            let h = hash_bytes(source.as_bytes());
            if self.hashes.get(&id) == Some(&h) {
                continue;
            }
            self.ingest(id.clone(), &source)?;
            self.hashes.insert(id.clone(), h);
            report.changed.push(id.to_string());
        }

        // Drop blocks whose files disappeared.
        let removed: Vec<BlockId> = self
            .vault
            .ids()
            .into_iter()
            .filter(|id| !seen.contains(id))
            .collect();
        for id in removed {
            self.vault.remove(&id);
            self.index.remove_block(&id)?;
            self.hashes.remove(&id);
            report.removed.push(id.to_string());
        }

        // After structural changes, links may have re-resolved; reindex touched neighbours is
        // overkill — a full link refresh keeps backlinks correct cheaply for a personal KB.
        if !report.is_empty() {
            self.refresh_links()?;
        }

        self.conflicts = conflicts.clone();
        report.conflicts = conflicts;
        Ok(report)
    }

    /// Rebuild the entire index from the `blocks/` directory (clear + re-ingest everything).
    pub fn rebuild(&mut self) -> Result<SyncReport, IndexError> {
        self.index.clear()?;
        self.vault = Vault::new();
        self.hashes.clear();
        self.conflicts.clear();
        self.reconcile()
    }

    /// Ingest one block (parse, index, embed). Does not touch disk.
    fn ingest(&mut self, id: BlockId, source: &str) -> Result<(), IndexError> {
        self.vault.insert_source(id.clone(), source);
        let block = self
            .vault
            .block(&id)
            .ok_or_else(|| IndexError::new("block vanished after insert"))?
            .clone();
        let child_count = self.vault.children(&id).len();
        let record = BlockRecord::from_block(&block, child_count);
        let links = block_links(&self.vault, &block);
        self.index.reindex_block(&record, &links)?;

        if let Some(embedder) = &self.embedder {
            let vector = embedder
                .embed_one(&record.contextual_text)
                .map_err(IndexError::new)?;
            let model_id = embedder.model_id();
            self.index.set_embedding(&id, &model_id, &vector)?;
        }
        Ok(())
    }

    /// Recompute link rows for every block (so backlinks reflect newly resolvable targets).
    fn refresh_links(&mut self) -> Result<(), IndexError> {
        for block in self.vault.blocks().into_iter().cloned().collect::<Vec<_>>() {
            let child_count = self.vault.children(&block.id).len();
            let record = BlockRecord::from_block(&block, child_count);
            let links = block_links(&self.vault, &block);
            self.index.reindex_block(&record, &links)?;
        }
        Ok(())
    }

    // ---------- writes ----------

    /// Create a new block from `body` (+ optional title), writing `blocks/<ulid>.md`. Returns
    /// the new block id.
    pub fn create_block(&mut self, title: Option<&str>, body: &str) -> Result<BlockId, IndexError> {
        let id = BlockId::generate();
        let source = write_block(title, body);
        self.write_file(&id, &source)?;
        self.ingest(id.clone(), &source)?;
        self.hashes
            .insert(id.clone(), hash_bytes(source.as_bytes()));
        self.refresh_links()?;
        Ok(id)
    }

    /// Overwrite a block's body (+ optional title), persisting to its file.
    pub fn update_block(
        &mut self,
        id: &BlockId,
        title: Option<&str>,
        body: &str,
    ) -> Result<(), IndexError> {
        if self.vault.block(id).is_none() {
            return Err(IndexError::new(format!("unknown block: {id}")));
        }
        let source = write_block(title, body);
        self.write_file(id, &source)?;
        self.ingest(id.clone(), &source)?;
        self.hashes
            .insert(id.clone(), hash_bytes(source.as_bytes()));
        self.refresh_links()?;
        Ok(())
    }

    /// Delete a block: remove its file and drop it from vault + index.
    pub fn delete_block(&mut self, id: &BlockId) -> Result<(), IndexError> {
        let abs = self.root.join(block_rel_path(id));
        if abs.exists() {
            std::fs::remove_file(&abs).map_err(io_err)?;
        }
        self.vault.remove(id);
        self.index.remove_block(id)?;
        self.hashes.remove(id);
        self.refresh_links()?;
        Ok(())
    }

    /// Append a directive to a block's body and persist. Used by linking.
    pub fn append_to_body(&mut self, id: &BlockId, suffix: &str) -> Result<(), IndexError> {
        let block = self
            .vault
            .block(id)
            .ok_or_else(|| IndexError::new(format!("unknown block: {id}")))?;
        let title = block.title.clone();
        let body = format!("{}\n\n{}\n", block.body.trim_end(), suffix);
        self.update_block(id, title.as_deref(), &body)
    }

    fn write_file(&self, id: &BlockId, source: &str) -> Result<(), IndexError> {
        let dir = self.blocks_dir();
        std::fs::create_dir_all(&dir).map_err(io_err)?;
        let abs = self.root.join(block_rel_path(id));
        std::fs::write(&abs, source).map_err(io_err)
    }

    /// Search the index, embedding the query text first when an embedder is attached.
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
}

/// Names of cloud-sync conflict copies in the `blocks/` directory (surfaced, never indexed).
fn conflict_file_names(root: &Path) -> Vec<String> {
    let dir = root.join(BLOCKS_DIR);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if crate::conflict::is_conflict_path(name) {
                    out.push(name.to_string());
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::testing::MemIndex;

    fn engine() -> (tempfile::TempDir, SyncEngine<MemIndex>) {
        let dir = tempfile::tempdir().unwrap();
        let engine = SyncEngine::new(dir.path(), MemIndex::default());
        (dir, engine)
    }

    #[test]
    fn create_writes_file_and_indexes() {
        let (dir, mut e) = engine();
        let id = e.create_block(Some("Title"), "the body\n").unwrap();
        assert!(dir.path().join(format!("blocks/{id}.md")).exists());
        assert_eq!(e.index().stats().unwrap().blocks, 1);
        assert_eq!(
            e.vault().block(&id).unwrap().title.as_deref(),
            Some("Title")
        );
    }

    #[test]
    fn update_persists_to_disk() {
        let (dir, mut e) = engine();
        let id = e.create_block(None, "original\n").unwrap();
        e.update_block(&id, None, "edited\n").unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join(format!("blocks/{id}.md"))).unwrap();
        assert!(on_disk.contains("edited"));
        assert!(!on_disk.contains("original"));

        // A fresh engine reading only from disk sees the edit.
        let mut e2 = SyncEngine::new(dir.path(), MemIndex::default());
        e2.reconcile().unwrap();
        assert_eq!(e2.vault().block(&id).unwrap().body, "edited\n");
    }

    #[test]
    fn reconcile_skips_unchanged_and_detects_deletion() {
        let (dir, mut e) = engine();
        let id = e.create_block(None, "x\n").unwrap();
        // Second reconcile: hash stable -> no change.
        let r = e.reconcile().unwrap();
        assert!(r.is_empty(), "unchanged dir should be a no-op: {r:?}");
        // Delete the file externally and reconcile.
        std::fs::remove_file(dir.path().join(format!("blocks/{id}.md"))).unwrap();
        let r = e.reconcile().unwrap();
        assert_eq!(r.removed, vec![id.to_string()]);
        assert_eq!(e.index().stats().unwrap().blocks, 0);
    }

    #[test]
    fn delete_block_removes_file_and_index() {
        let (dir, mut e) = engine();
        let id = e.create_block(None, "x\n").unwrap();
        e.delete_block(&id).unwrap();
        assert!(!dir.path().join(format!("blocks/{id}.md")).exists());
        assert_eq!(e.index().stats().unwrap().blocks, 0);
    }
}
