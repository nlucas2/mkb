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
use crate::vault::{
    block_rel_path, read_block_files, sanitize_asset_filename, Vault, ASSETS_DIR, BLOCKS_DIR,
};

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
    /// the new block id. Frontmatter tags start empty; inline `#tags` in the body still apply.
    pub fn create_block(&mut self, title: Option<&str>, body: &str) -> Result<BlockId, IndexError> {
        let id = BlockId::generate();
        let now = crate::clock::now_rfc3339();
        let source = write_block(title, &[], false, Some(now.as_str()), &[], body);
        self.write_file(&id, &source)?;
        self.ingest(id.clone(), &source)?;
        self.hashes
            .insert(id.clone(), hash_bytes(source.as_bytes()));
        self.refresh_links()?;
        Ok(id)
    }

    /// Write binary asset `bytes` into the vault's `assets/` directory under a safe filename
    /// derived from `suggested_name`, returning the **vault-relative** path (e.g. `assets/x.png`)
    /// to drop into a Markdown image. The name is sanitised to a single safe component and made
    /// unique against existing files (`name.ext`, then `name-1.ext`, …) so an import never
    /// silently overwrites an existing asset. Assets are not indexed — this just lands a file in
    /// the vault for a block to reference.
    pub fn add_asset(&self, suggested_name: &str, bytes: &[u8]) -> Result<String, IndexError> {
        let dir = self.root.join(ASSETS_DIR);
        std::fs::create_dir_all(&dir).map_err(io_err)?;
        let safe = sanitize_asset_filename(suggested_name);
        let (stem, ext) = match safe.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), Some(e.to_string())),
            None => (safe.clone(), None),
        };
        let mut name = safe.clone();
        let mut n = 0u32;
        while dir.join(&name).exists() {
            n += 1;
            name = match &ext {
                Some(ext) => format!("{stem}-{n}.{ext}"),
                None => format!("{stem}-{n}"),
            };
        }
        std::fs::write(dir.join(&name), bytes).map_err(io_err)?;
        Ok(format!("{ASSETS_DIR}/{name}"))
    }

    /// List **orphaned assets**: files under the vault's `assets/` directory that no block
    /// references. Each is returned as a vault-relative path (e.g. `assets/old.png`), sorted.
    ///
    /// Reference detection is deliberately conservative — an asset counts as referenced if *any*
    /// block body contains its path as a substring (so `![](assets/x.png)`, `![](./assets/x.png)`
    /// and `[](/assets/x.png)` all keep it). This errs toward keeping a file; cleanup is never
    /// automatic. Only `assets/` is scanned, so unrelated files elsewhere in the vault are untouched.
    pub fn orphan_assets(&self) -> Vec<String> {
        let mut assets = Vec::new();
        collect_files_rel(&self.root.join(ASSETS_DIR), ASSETS_DIR, &mut assets);
        let bodies: Vec<&str> = self
            .vault
            .blocks()
            .iter()
            .map(|b| b.body.as_str())
            .collect();
        let mut orphans: Vec<String> = assets
            .into_iter()
            .filter(|rel| !bodies.iter().any(|body| body.contains(rel.as_str())))
            .collect();
        orphans.sort();
        orphans
    }

    /// Delete an asset by its vault-relative path (as returned by [`SyncEngine::orphan_assets`]).
    /// The path is confined to the vault's `assets/` directory: traversal is rejected and any path
    /// outside `assets/` is refused, so this can only remove files mdkb manages as assets.
    pub fn remove_asset(&self, rel: &str) -> Result<(), IndexError> {
        let clean = crate::vault::safe_relative_path(rel).map_err(io_err)?;
        let prefix = format!("{ASSETS_DIR}/");
        if !clean.starts_with(&prefix) {
            return Err(io_err(format!(
                "not an asset path (must be under {ASSETS_DIR}/): {rel}"
            )));
        }
        let abs = self.root.join(&clean);
        match std::fs::remove_file(&abs) {
            Ok(()) => Ok(()),
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_err(e)),
        }
    }
    /// Overwrite a block's body, persisting to its file, and optionally change its title. The
    /// block's managed frontmatter `tags:`, its `locked:` flag, and its arbitrary `props` are
    /// **preserved** across the edit (each is changed via its own op — [`SyncEngine::set_tags`] /
    /// [`SyncEngine::set_lock`] / [`SyncEngine::set_props`] — not by a body edit, so editing prose
    /// never drops tags, silently unlocks a block, or strips its metadata).
    ///
    /// `title` follows the same preserve-by-default rule: `None` keeps the existing title, `Some`
    /// with a non-empty value sets it, and `Some("")` (an explicit empty title) clears it. So a
    /// body-only edit never drops the title.
    pub fn update_block(
        &mut self,
        id: &BlockId,
        title: Option<&str>,
        body: &str,
    ) -> Result<(), IndexError> {
        let existing = self
            .vault
            .block(id)
            .ok_or_else(|| IndexError::new(format!("unknown block: {id}")))?;
        let fm_tags = existing.fm_tags.clone();
        let locked = existing.locked;
        let props = existing.props.clone();
        // Preserve the existing title unless the caller passed one. An explicit empty string
        // clears it; otherwise `None` keeps whatever the block already had.
        let resolved_title: Option<String> = match title {
            None => existing.title.clone(),
            Some(t) if t.trim().is_empty() => None,
            Some(t) => Some(t.to_string()),
        };
        let now = crate::clock::now_rfc3339();
        let source = write_block(
            resolved_title.as_deref(),
            &fm_tags,
            locked,
            Some(now.as_str()),
            &props,
            body,
        );
        self.write_file(id, &source)?;
        self.ingest(id.clone(), &source)?;
        self.hashes
            .insert(id.clone(), hash_bytes(source.as_bytes()));
        self.refresh_links()?;
        Ok(())
    }

    /// Set a block's **managed** (frontmatter) tags to exactly `tags`, preserving its title and
    /// body. Inline `#hashtag` mentions in the body are untouched. Tag names are trimmed and
    /// de-duplicated (case-insensitive), preserving first-seen order.
    pub fn set_tags(&mut self, id: &BlockId, tags: &[String]) -> Result<(), IndexError> {
        let existing = self
            .vault
            .block(id)
            .ok_or_else(|| IndexError::new(format!("unknown block: {id}")))?;
        let title = existing.title.clone();
        let body = existing.body.clone();
        let locked = existing.locked;
        let props = existing.props.clone();
        let mut clean: Vec<String> = Vec::new();
        for t in tags {
            let t = t.trim();
            if !t.is_empty() && !clean.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                clean.push(t.to_string());
            }
        }
        let now = crate::clock::now_rfc3339();
        let source = write_block(
            title.as_deref(),
            &clean,
            locked,
            Some(now.as_str()),
            &props,
            &body,
        );
        self.write_file(id, &source)?;
        self.ingest(id.clone(), &source)?;
        self.hashes
            .insert(id.clone(), hash_bytes(source.as_bytes()));
        self.refresh_links()?;
        Ok(())
    }

    /// Set a block's `locked:` flag to exactly `locked`, preserving its title, managed tags, props,
    /// and body. This is the only writer that changes lock state; body/tag edits preserve it.
    pub fn set_lock(&mut self, id: &BlockId, locked: bool) -> Result<(), IndexError> {
        let existing = self
            .vault
            .block(id)
            .ok_or_else(|| IndexError::new(format!("unknown block: {id}")))?;
        let title = existing.title.clone();
        let tags = existing.fm_tags.clone();
        let body = existing.body.clone();
        let props = existing.props.clone();
        let now = crate::clock::now_rfc3339();
        let source = write_block(
            title.as_deref(),
            &tags,
            locked,
            Some(now.as_str()),
            &props,
            &body,
        );
        self.write_file(id, &source)?;
        self.ingest(id.clone(), &source)?;
        self.hashes
            .insert(id.clone(), hash_bytes(source.as_bytes()));
        self.refresh_links()?;
        Ok(())
    }

    /// **Merge** properties into a block: each `(key, value)` in `props` is added (or updates the
    /// existing value for that key, case-insensitively); **every other property is preserved**.
    /// Title, managed tags, the `locked:` flag, and the body are untouched. This is deliberately
    /// add/update-only — there is no replace-the-whole-set operation, so a caller (especially an
    /// agent) can never silently drop a property it didn't name; use [`SyncEngine::unset_props`] to
    /// remove. Keys are trimmed and validated (rejecting managed names, malformed keys, and empty
    /// values); within `props`, the first value for a duplicated key wins.
    pub fn set_props(
        &mut self,
        id: &BlockId,
        props: &[(String, String)],
    ) -> Result<(), IndexError> {
        let existing = self
            .vault
            .block(id)
            .ok_or_else(|| IndexError::new(format!("unknown block: {id}")))?;
        let title = existing.title.clone();
        let tags = existing.fm_tags.clone();
        let locked = existing.locked;
        let body = existing.body.clone();
        // Start from the block's current properties and upsert — never replace the whole set.
        let mut merged: Vec<(String, String)> = existing.props.clone();
        let mut seen: Vec<String> = Vec::new();
        for (k, v) in props {
            let k = k.trim();
            if k.is_empty() {
                continue;
            }
            // Reject malformed keys before writing anything: a key with a newline, `:`, or
            // whitespace would inject extra frontmatter lines (e.g. a smuggled `locked: true`).
            if !crate::blockfile::is_prop_key(k) {
                return Err(IndexError::new(format!(
                    "invalid property key {k:?}: keys must start with a letter and contain only \
                     letters, digits, '_' or '-'"
                )));
            }
            // Reject keys mdkb manages itself — properties must not shadow title/tags/locked
            // (locked is a human-only flag agents cannot set via the property path).
            if crate::blockfile::is_managed_key(k) {
                return Err(IndexError::new(format!(
                    "reserved property key {k:?}: title, tags, and locked are managed by mdkb and \
                     cannot be set as properties"
                )));
            }
            // Reject empty values: an empty scalar is dropped on re-parse, so accepting it would
            // break the parse/write symmetry. To remove a property, use unset_props.
            if v.trim().is_empty() {
                return Err(IndexError::new(format!(
                    "empty value for property {k:?}: use unset to remove a property, don't set it empty"
                )));
            }
            // First value wins for a key duplicated within this call.
            if seen.iter().any(|x| x.eq_ignore_ascii_case(k)) {
                continue;
            }
            seen.push(k.to_string());
            // Upsert: update in place if the key already exists, else append (preserving order).
            if let Some(slot) = merged.iter_mut().find(|(x, _)| x.eq_ignore_ascii_case(k)) {
                slot.1 = v.clone();
            } else {
                merged.push((k.to_string(), v.clone()));
            }
        }
        let now = crate::clock::now_rfc3339();
        let source = write_block(
            title.as_deref(),
            &tags,
            locked,
            Some(now.as_str()),
            &merged,
            &body,
        );
        self.write_file(id, &source)?;
        self.ingest(id.clone(), &source)?;
        self.hashes
            .insert(id.clone(), hash_bytes(source.as_bytes()));
        self.refresh_links()?;
        Ok(())
    }

    /// Remove the named properties from a block, preserving every other property as well as the
    /// title, managed tags, `locked:` flag, and body. Keys are matched case-insensitively; keys
    /// that aren't present are ignored (the call is idempotent). This is the only way to drop a
    /// property — there is no replace-the-whole-set operation.
    pub fn unset_props(&mut self, id: &BlockId, keys: &[String]) -> Result<(), IndexError> {
        let existing = self
            .vault
            .block(id)
            .ok_or_else(|| IndexError::new(format!("unknown block: {id}")))?;
        let title = existing.title.clone();
        let tags = existing.fm_tags.clone();
        let locked = existing.locked;
        let body = existing.body.clone();
        let drop: Vec<&str> = keys.iter().map(|k| k.trim()).collect();
        let kept: Vec<(String, String)> = existing
            .props
            .iter()
            .filter(|(k, _)| !drop.iter().any(|d| d.eq_ignore_ascii_case(k)))
            .cloned()
            .collect();
        let now = crate::clock::now_rfc3339();
        let source = write_block(
            title.as_deref(),
            &tags,
            locked,
            Some(now.as_str()),
            &kept,
            &body,
        );
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
                // Double quotes are an FTS phrase directive, not content — strip them so the
                // semantic vector embeds the natural-language text, not the quote characters.
                let clean = text.replace('"', " ");
                let vector = embedder.embed_one(&clean).map_err(IndexError::new)?;
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

/// Recursively collect files under `dir`, returning each as a forward-slash path prefixed with
/// `rel_prefix` (the vault-relative directory). Used to enumerate `assets/` for the orphan sweep.
/// A missing directory yields nothing; symlinks are followed only as the OS metadata reports.
fn collect_files_rel(dir: &Path, rel_prefix: &str, out: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let rel = format!("{rel_prefix}/{name}");
        if entry.path().is_dir() {
            collect_files_rel(&entry.path(), &rel, out);
        } else {
            out.push(rel);
        }
    }
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
    fn add_asset_writes_file_and_uniquifies() {
        let (dir, e) = engine();
        let rel = e.add_asset("diagram.png", b"PNGDATA").unwrap();
        assert_eq!(rel, "assets/diagram.png");
        let p = dir.path().join(&rel);
        assert_eq!(std::fs::read(&p).unwrap(), b"PNGDATA");
        // A second import of the same name does not overwrite — it gets a unique suffix.
        let rel2 = e.add_asset("diagram.png", b"OTHER").unwrap();
        assert_eq!(rel2, "assets/diagram-1.png");
        assert_eq!(std::fs::read(dir.path().join(&rel)).unwrap(), b"PNGDATA");
        // A path-like name is reduced to a safe filename inside assets/ (no traversal).
        let rel3 = e.add_asset("../../evil.sh", b"x").unwrap();
        assert_eq!(rel3, "assets/evil.sh");
        assert!(dir.path().join("assets/evil.sh").exists());
        // assets/ is not indexed.
        assert_eq!(e.index().stats().unwrap().blocks, 0);
    }

    #[test]
    fn orphan_assets_lists_unreferenced_and_remove_deletes() {
        let (dir, mut e) = engine();
        let used = e.add_asset("used.png", b"a").unwrap();
        let orphan = e.add_asset("orphan.png", b"b").unwrap();
        // A block references `used.png` (relative form with ./ still counts).
        e.create_block(None, &format!("see ![pic](./{used})\n"))
            .unwrap();

        let orphans = e.orphan_assets();
        assert_eq!(
            orphans,
            vec![orphan.clone()],
            "only the unreferenced asset is an orphan"
        );

        // Removing the orphan deletes the file; the referenced one stays.
        e.remove_asset(&orphan).unwrap();
        assert!(!dir.path().join(&orphan).exists());
        assert!(dir.path().join(&used).exists());
        assert!(e.orphan_assets().is_empty());

        // Confinement: a non-asset / traversal path is refused.
        assert!(e.remove_asset("blocks/whatever.md").is_err());
        assert!(e.remove_asset("../secret").is_err());
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
    fn update_preserves_title_unless_explicitly_changed() {
        let (_dir, mut e) = engine();
        let id = e.create_block(Some("Keep Me"), "original\n").unwrap();
        // Body-only edit (title None) must NOT drop the title.
        e.update_block(&id, None, "edited\n").unwrap();
        assert_eq!(
            e.vault().block(&id).unwrap().title.as_deref(),
            Some("Keep Me")
        );
        // A non-empty title sets it.
        e.update_block(&id, Some("New Title"), "edited2\n").unwrap();
        assert_eq!(
            e.vault().block(&id).unwrap().title.as_deref(),
            Some("New Title")
        );
        // An explicit empty title clears it.
        e.update_block(&id, Some(""), "edited3\n").unwrap();
        assert_eq!(e.vault().block(&id).unwrap().title, None);
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

    #[test]
    fn set_tags_persists_and_body_edits_preserve_them() {
        let (dir, mut e) = engine();
        let id = e.create_block(Some("Note"), "body\n").unwrap();
        // Set managed tags.
        e.set_tags(&id, &["k8s".to_string(), "ops".to_string()])
            .unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join(format!("blocks/{id}.md"))).unwrap();
        assert!(on_disk.contains("tags: [k8s, ops]"), "got:\n{on_disk}");
        assert_eq!(e.vault().block(&id).unwrap().fm_tags, vec!["k8s", "ops"]);

        // A plain body edit must NOT drop the managed tags (the historical bug).
        e.update_block(&id, Some("Note"), "edited body\n").unwrap();
        let after = std::fs::read_to_string(dir.path().join(format!("blocks/{id}.md"))).unwrap();
        assert!(after.contains("tags: [k8s, ops]"), "tags dropped:\n{after}");
        assert_eq!(e.vault().block(&id).unwrap().fm_tags, vec!["k8s", "ops"]);
        assert_eq!(e.vault().block(&id).unwrap().body, "edited body\n");
    }

    #[test]
    fn set_tags_dedupes_and_can_clear() {
        let (dir, mut e) = engine();
        let id = e.create_block(None, "b\n").unwrap();
        e.set_tags(&id, &["a".into(), "A".into(), "b".into()])
            .unwrap();
        assert_eq!(e.vault().block(&id).unwrap().fm_tags, vec!["a", "b"]);
        // Clearing removes the frontmatter entirely (pure body again).
        e.set_tags(&id, &[]).unwrap();
        assert!(e.vault().block(&id).unwrap().fm_tags.is_empty());
        let on_disk = std::fs::read_to_string(dir.path().join(format!("blocks/{id}.md"))).unwrap();
        assert!(
            !on_disk.contains("tags:"),
            "tags should be gone:\n{on_disk}"
        );
    }

    #[test]
    fn set_props_persists_and_body_edits_preserve_them() {
        let (dir, mut e) = engine();
        let id = e.create_block(Some("Atom"), "body\n").unwrap();
        e.set_props(
            &id,
            &[
                ("source".to_string(), "https://example.com/x".to_string()),
                ("verified".to_string(), "2026-06-01".to_string()),
            ],
        )
        .unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join(format!("blocks/{id}.md"))).unwrap();
        assert!(
            on_disk.contains("source: https://example.com/x"),
            "got:\n{on_disk}"
        );
        assert_eq!(
            e.vault().block(&id).unwrap().prop("verified"),
            Some("2026-06-01")
        );

        // The historical bug class: a plain body edit must NOT drop the properties.
        e.update_block(&id, Some("Atom"), "edited body\n").unwrap();
        let after = std::fs::read_to_string(dir.path().join(format!("blocks/{id}.md"))).unwrap();
        assert!(
            after.contains("source: https://example.com/x"),
            "props dropped:\n{after}"
        );
        assert_eq!(
            e.vault().block(&id).unwrap().prop("source"),
            Some("https://example.com/x")
        );
        assert_eq!(e.vault().block(&id).unwrap().body, "edited body\n");

        // A tag edit must also preserve the properties (and vice-versa).
        e.set_tags(&id, &["mem".to_string()]).unwrap();
        let b = e.vault().block(&id).unwrap();
        assert_eq!(b.fm_tags, vec!["mem"]);
        assert_eq!(b.prop("verified"), Some("2026-06-01"));
    }

    #[test]
    fn set_props_merges_and_dedupes_within_call() {
        let (_dir, mut e) = engine();
        let id = e.create_block(None, "b\n").unwrap();
        e.set_props(
            &id,
            &[
                ("k".into(), "1".into()),
                ("K".into(), "2".into()), // duplicate key (case-insensitive) -> first wins
                ("other".into(), "3".into()),
            ],
        )
        .unwrap();
        assert_eq!(
            e.vault().block(&id).unwrap().props.clone(),
            vec![
                ("k".to_string(), "1".to_string()),
                ("other".to_string(), "3".to_string())
            ]
        );
    }

    #[test]
    fn set_props_is_merge_not_replace() {
        // The whole point: setting one key must NOT drop the others (no clobber).
        let (_dir, mut e) = engine();
        let id = e.create_block(None, "b\n").unwrap();
        e.set_props(
            &id,
            &[
                ("source".into(), "git".into()),
                ("verified".into(), "2026-01-01".into()),
            ],
        )
        .unwrap();
        // A later call naming only `verified` updates it and PRESERVES `source`.
        e.set_props(&id, &[("verified".into(), "2026-06-01".into())])
            .unwrap();
        let b = e.vault().block(&id).unwrap();
        assert_eq!(
            b.prop("source"),
            Some("git"),
            "other prop must be preserved"
        );
        assert_eq!(b.prop("verified"), Some("2026-06-01"), "named prop updated");
        // Adding a brand-new key keeps the existing two.
        e.set_props(&id, &[("confidence".into(), "0.9".into())])
            .unwrap();
        assert_eq!(e.vault().block(&id).unwrap().props.len(), 3);
    }

    #[test]
    fn unset_props_removes_only_named_keys() {
        let (dir, mut e) = engine();
        let id = e.create_block(None, "b\n").unwrap();
        e.set_props(
            &id,
            &[
                ("source".into(), "git".into()),
                ("verified".into(), "2026-06-01".into()),
                ("confidence".into(), "0.9".into()),
            ],
        )
        .unwrap();
        // Remove one (case-insensitively); the others stay.
        e.unset_props(&id, &["Verified".into()]).unwrap();
        let b = e.vault().block(&id).unwrap();
        assert_eq!(b.prop("verified"), None);
        assert_eq!(b.prop("source"), Some("git"));
        assert_eq!(b.prop("confidence"), Some("0.9"));
        // Removing an unknown key is a no-op (idempotent).
        e.unset_props(&id, &["nope".into()]).unwrap();
        assert_eq!(e.vault().block(&id).unwrap().props.len(), 2);
        // Removing the rest clears the frontmatter back to a pure body.
        e.unset_props(&id, &["source".into(), "confidence".into()])
            .unwrap();
        assert!(e.vault().block(&id).unwrap().props.is_empty());
        let on_disk = std::fs::read_to_string(dir.path().join(format!("blocks/{id}.md"))).unwrap();
        assert!(
            !on_disk.contains("source:"),
            "props should be gone:\n{on_disk}"
        );
    }

    #[test]
    fn set_props_rejects_injection_key_and_writes_nothing() {
        let (dir, mut e) = engine();
        let id = e.create_block(Some("Victim"), "body\n").unwrap();
        // A key smuggling a newline + `locked: true` must be refused — agents cannot set the
        // human-only lock flag (that needs the app-only ManageLocks capability).
        let err = e.set_props(&id, &[("evil\nlocked".to_string(), "true".to_string())]);
        assert!(err.is_err(), "injection key must be rejected");
        let b = e.vault().block(&id).unwrap();
        assert!(!b.locked, "block must remain unlocked");
        assert!(b.props.is_empty(), "no property should have been written");
        let on_disk = std::fs::read_to_string(dir.path().join(format!("blocks/{id}.md"))).unwrap();
        assert!(!on_disk.contains("locked"), "lock flag leaked:\n{on_disk}");
    }

    #[test]
    fn set_props_rejects_managed_key_names() {
        // Even a syntactically-clean key must be refused if it shadows a managed key — otherwise
        // `set-props locked=true` would lock a block, bypassing the app-only ManageLocks gate.
        let (dir, mut e) = engine();
        let id = e.create_block(Some("Victim"), "body\n").unwrap();
        for key in ["locked", "Locked", "title", "tags"] {
            let err = e.set_props(&id, &[(key.to_string(), "true".to_string())]);
            assert!(err.is_err(), "managed key {key:?} must be rejected");
        }
        let b = e.vault().block(&id).unwrap();
        assert!(!b.locked, "block must remain unlocked");
        assert!(b.props.is_empty());
        let on_disk = std::fs::read_to_string(dir.path().join(format!("blocks/{id}.md"))).unwrap();
        assert!(!on_disk.contains("locked"), "lock flag leaked:\n{on_disk}");
        // The original title is intact (no injected override).
        assert_eq!(b.title.as_deref(), Some("Victim"));
    }

    #[test]
    fn set_props_rejects_empty_value() {
        let (_dir, mut e) = engine();
        let id = e.create_block(None, "body\n").unwrap();
        // An empty value would be dropped on re-parse; reject it to keep parse/write symmetric.
        assert!(e
            .set_props(&id, &[("note".to_string(), "  ".to_string())])
            .is_err());
        assert!(e.vault().block(&id).unwrap().props.is_empty());
    }

    #[test]
    fn set_props_rejects_timestamp_keys() {
        // created/updated are system-owned; a caller can't set them via a property.
        let (_dir, mut e) = engine();
        let id = e.create_block(None, "b\n").unwrap();
        assert!(e
            .set_props(&id, &[("created".into(), "2020-01-01".into())])
            .is_err());
        assert!(e
            .set_props(&id, &[("updated".into(), "2020-01-01".into())])
            .is_err());
        assert!(e.vault().block(&id).unwrap().props.is_empty());
    }

    #[test]
    fn writes_stamp_updated_and_created_comes_from_id() {
        let (_dir, mut e) = engine();
        let id = e.create_block(Some("A"), "body\n").unwrap();
        let b = e.vault().block(&id).unwrap();
        assert!(b.updated.is_some(), "create should stamp updated");
        assert!(b.created().is_some(), "created derives from the ULID id");
        let first = b.updated.clone();
        // A later edit re-stamps updated (monotonic; equal is fine within the same second).
        e.update_block(&id, Some("A"), "edited\n").unwrap();
        let b2 = e.vault().block(&id).unwrap();
        assert!(b2.updated.as_deref() >= first.as_deref());
        // The on-disk file carries `updated:` but never `created:`.
        let on_disk = std::fs::read_to_string(_dir.path().join(format!("blocks/{id}.md"))).unwrap();
        assert!(on_disk.contains("updated:"), "{on_disk}");
        assert!(!on_disk.contains("created:"), "{on_disk}");
    }

    #[test]
    fn pre_feature_block_has_no_updated_then_gets_stamped_on_edit() {
        // The survival case the user flagged: an existing file with no `updated:` must read fine,
        // and only gain a timestamp when it's next edited (never a mass-rewrite of old blocks).
        let (dir, mut e) = engine();
        let id = BlockId::generate();
        std::fs::create_dir_all(dir.path().join("blocks")).unwrap();
        std::fs::write(
            dir.path().join(format!("blocks/{id}.md")),
            "---\ntitle: Old\n---\n\nbody\n",
        )
        .unwrap();
        e.reconcile().unwrap();
        assert_eq!(
            e.vault().block(&id).unwrap().updated,
            None,
            "old block has no updated until edited"
        );
        e.set_tags(&id, &["t".into()]).unwrap();
        assert!(
            e.vault().block(&id).unwrap().updated.is_some(),
            "an edit stamps updated"
        );
    }
}
