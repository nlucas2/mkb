//! The vault: a collection of Markdown pages forming the knowledge base.
//!
//! A [`Vault`] can be built in memory (great for tests) or loaded from a directory of
//! `.md` files. It owns the parsed [`Document`]s, resolves page names and block ids, and
//! performs id assignment and block edits while preserving file fidelity.

use crate::block::Block;
use crate::document::Document;
use crate::id::{BlockId, IdCodec, NativeIdCodec};
use std::collections::HashMap;
use std::io;
use std::path::Path;

/// A single page: its vault-relative path (forward-slash, no `./`) and parsed document.
#[derive(Debug, Clone)]
pub struct Page {
    /// Vault-relative path including the `.md` extension, using `/` separators.
    pub path: String,
    /// The parsed document.
    pub doc: Document,
}

impl Page {
    /// The page "name": the file stem (filename without directory or `.md`).
    pub fn name(&self) -> &str {
        let file = self.path.rsplit('/').next().unwrap_or(&self.path);
        file.strip_suffix(".md").unwrap_or(file)
    }
}

/// An in-memory collection of pages with name/id resolution.
#[derive(Debug, Clone, Default)]
pub struct Vault {
    pages: Vec<Page>,
    by_path: HashMap<String, usize>,
    by_stem: HashMap<String, Vec<usize>>,
    by_block: HashMap<BlockId, usize>,
}

impl Vault {
    /// An empty vault.
    pub fn new() -> Self {
        Vault::default()
    }

    /// Number of pages.
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// Whether the vault has no pages.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// All pages, in insertion order.
    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    /// Insert (or replace) a page from raw Markdown source.
    pub fn insert(&mut self, path: impl Into<String>, source: impl Into<String>) {
        let path = normalize_path(&path.into());
        let doc = Document::parse(source.into());
        match self.by_path.get(&key(&path)).copied() {
            Some(i) => self.pages[i] = Page { path, doc },
            None => self.pages.push(Page { path, doc }),
        }
        self.reindex();
    }

    /// Load every `.md` file under `root` (recursively) into a vault.
    pub fn from_dir(root: impl AsRef<Path>) -> io::Result<Vault> {
        let root = root.as_ref();
        let mut vault = Vault::new();
        for (rel, abs) in markdown_files(root)? {
            let source = std::fs::read_to_string(&abs)?;
            vault.insert(rel, source);
        }
        Ok(vault)
    }

    /// Remove a page from the in-memory vault (does not touch the filesystem).
    pub fn remove(&mut self, path: &str) -> bool {
        let k = key(&normalize_path(path));
        match self.by_path.get(&k).copied() {
            Some(i) => {
                self.pages.remove(i);
                self.reindex();
                true
            }
            None => false,
        }
    }

    /// Resolve a page by name or relative path (case-insensitive).
    pub fn page(&self, key_or_name: &str) -> Option<&Page> {
        let normalized = normalize_path(key_or_name);
        // Try as a path, with and without `.md`.
        let candidates = [key(&normalized), key(&format!("{normalized}.md"))];
        for c in candidates {
            if let Some(&i) = self.by_path.get(&c) {
                return Some(&self.pages[i]);
            }
        }
        // Fall back to a unique stem match.
        let stem = key(normalized.strip_suffix(".md").unwrap_or(&normalized));
        let stem = stem.rsplit('/').next().unwrap_or(&stem).to_string();
        match self.by_stem.get(&stem) {
            Some(v) if v.len() == 1 => Some(&self.pages[v[0]]),
            _ => None,
        }
    }

    /// Find the page and block for a given block id.
    pub fn block(&self, id: &BlockId) -> Option<(&Page, &Block)> {
        let &i = self.by_block.get(id)?;
        let page = &self.pages[i];
        let block = page.doc.block(id)?;
        Some((page, block))
    }

    /// Replace the textual content of a block (identified by id) with `new_text`.
    ///
    /// The change is spliced into the page source so the rest of the file — including the
    /// block's id marker — is preserved, then the page is re-parsed. Returns the new page
    /// source on success, or `None` if the id is unknown.
    pub fn update_block(&mut self, id: &BlockId, new_text: &str) -> Option<String> {
        let &i = self.by_block.get(id)?;
        let range = self.pages[i].doc.block(id)?.content_range.clone();
        let mut source = self.pages[i].doc.source.clone();
        source.replace_range(range, new_text);
        self.pages[i].doc = Document::parse(source.clone());
        self.reindex();
        Some(source)
    }

    /// Assign ids to every block lacking one, across all pages, using the native codec.
    /// Returns the set of pages whose source changed, as `(path, new_source)`.
    pub fn assign_ids(&mut self) -> Vec<(String, String)> {
        self.assign_ids_with(&NativeIdCodec)
    }

    /// As [`Vault::assign_ids`] but with an explicit codec.
    pub fn assign_ids_with(&mut self, codec: &dyn IdCodec) -> Vec<(String, String)> {
        let mut changed = Vec::new();
        for page in &mut self.pages {
            if let Some(new_source) = page.doc.with_assigned_ids(codec) {
                page.doc = Document::parse_with(codec, new_source.clone());
                changed.push((page.path.clone(), new_source));
            }
        }
        self.reindex();
        changed
    }

    fn reindex(&mut self) {
        self.by_path.clear();
        self.by_stem.clear();
        self.by_block.clear();
        for (i, page) in self.pages.iter().enumerate() {
            self.by_path.insert(key(&page.path), i);
            self.by_stem.entry(key(page.name())).or_default().push(i);
            for b in &page.doc.blocks {
                self.by_block.insert(b.id.clone(), i);
            }
        }
    }
}

fn normalize_path(p: &str) -> String {
    p.replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

fn key(s: &str) -> String {
    s.to_lowercase()
}

/// Recursively list `.md` files under `root`, returning `(vault-relative path, absolute
/// path)` pairs. Hidden directories (those starting with `.`) are skipped.
pub fn markdown_files(root: impl AsRef<Path>) -> io::Result<Vec<(String, std::path::PathBuf)>> {
    let root = root.as_ref();
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                if !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
                {
                    stack.push(p);
                }
            } else if p.extension().and_then(|e| e.to_str()) == Some("md") {
                let rel = p.strip_prefix(root).unwrap_or(&p);
                let rel = rel.to_string_lossy().replace('\\', "/");
                out.push((rel, p));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_pages_by_name_and_path() {
        let mut v = Vault::new();
        v.insert("topic/useful-queries.md", "# Queries\n");
        assert!(v.page("useful-queries").is_some());
        assert!(v.page("Useful-Queries").is_some());
        assert!(v.page("topic/useful-queries").is_some());
        assert!(v.page("topic/useful-queries.md").is_some());
        assert!(v.page("missing").is_none());
    }

    #[test]
    fn assign_ids_then_resolve_blocks() {
        let mut v = Vault::new();
        v.insert("a.md", "first\n\nsecond\n");
        let changed = v.assign_ids();
        assert_eq!(changed.len(), 1);
        // Every block is now resolvable by id.
        let page = v.page("a").unwrap();
        for b in page.doc.blocks.clone() {
            assert!(v.block(&b.id).is_some());
        }
        // Idempotent: a second pass changes nothing.
        assert!(v.assign_ids().is_empty());
    }

    #[test]
    fn update_block_preserves_id_and_changes_text() {
        let mut v = Vault::new();
        v.insert("a.md", "hello world\n");
        v.assign_ids();
        let id = v.page("a").unwrap().doc.blocks[0].id.clone();
        v.update_block(&id, "goodbye world").unwrap();
        let (_, block) = v.block(&id).unwrap();
        assert_eq!(block.content, "goodbye world");
        assert_eq!(block.id, id, "id must survive an edit");
    }
}
