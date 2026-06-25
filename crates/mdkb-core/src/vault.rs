//! The vault: the collection of block files that form the knowledge base.
//!
//! In the file-per-block model a [`Vault`] is a map of [`BlockId`] → [`Block`], one entry per
//! `blocks/<ulid>.md` file. Edges are derived from each block's directives: `![[target]]`
//! (children / transclusions) and `[[target]]` (references). A *target* is resolved to a
//! concrete id by ULID first, then by an exact (case-insensitive) title match.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::Path;

use crate::block::Block;
use crate::blockfile::parse_block;
use crate::id::BlockId;

/// The directory (relative to the vault root) that holds block files.
pub const BLOCKS_DIR: &str = "blocks";

/// The directory (relative to the vault root) that holds non-block assets — images and other
/// files a block references with a normal Markdown `![](assets/…)` / `[](assets/…)` link. Assets
/// are carried by sync but never indexed (only `BLOCKS_DIR` is scanned for content).
pub const ASSETS_DIR: &str = "assets";

/// A block referenced by id + its display title — a single step in a breadcrumb.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Crumb {
    /// The block's id.
    pub id: BlockId,
    /// The block's display title (falls back to the id when untitled).
    pub title: String,
}

/// The upward context of a block: where it sits in the transclusion DAG. Lets a search hit on a
/// nested, embedded block tell the user which page(s) it actually lives on, instead of presenting
/// it as an isolated fragment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Lineage {
    /// Whether the block is a root (transcluded by nothing — a top-level page).
    pub is_root: bool,
    /// Fewest embed-hops from this block up to its nearest root (`0` for a root).
    pub depth: usize,
    /// The blocks that directly transclude this one (`![[id]]`).
    pub parents: Vec<Crumb>,
    /// The distinct root pages reachable by walking embed edges upward (the block itself if root).
    pub roots: Vec<Crumb>,
}

/// An in-memory collection of block files with id/title resolution.
#[derive(Debug, Clone, Default)]
pub struct Vault {
    blocks: HashMap<BlockId, Block>,
    by_title: HashMap<String, BlockId>,
}

impl Vault {
    /// An empty vault.
    pub fn new() -> Self {
        Vault::default()
    }

    /// Number of blocks.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether the vault has no blocks.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Insert (or replace) a block from its raw file source.
    pub fn insert_source(&mut self, id: BlockId, source: &str) {
        let block = parse_block(id, source);
        self.insert(block);
    }

    /// Insert (or replace) a parsed block.
    pub fn insert(&mut self, block: Block) {
        self.blocks.insert(block.id.clone(), block);
        self.reindex_titles();
    }

    /// Remove a block by id. Returns whether it existed.
    pub fn remove(&mut self, id: &BlockId) -> bool {
        let existed = self.blocks.remove(id).is_some();
        if existed {
            self.reindex_titles();
        }
        existed
    }

    /// Fetch a block by id.
    pub fn block(&self, id: &BlockId) -> Option<&Block> {
        self.blocks.get(id)
    }

    /// All blocks, in a stable (id-sorted) order.
    pub fn blocks(&self) -> Vec<&Block> {
        let mut v: Vec<&Block> = self.blocks.values().collect();
        v.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        v
    }

    /// All block ids, sorted.
    pub fn ids(&self) -> Vec<BlockId> {
        let mut v: Vec<BlockId> = self.blocks.keys().cloned().collect();
        v.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        v
    }

    /// Resolve a directive target (a ULID or a title) to a concrete block id.
    pub fn resolve(&self, target: &str) -> Option<BlockId> {
        let t = target.trim();
        if let Ok(id) = BlockId::parse(t) {
            if self.blocks.contains_key(&id) {
                return Some(id);
            }
        }
        self.by_title.get(&t.to_lowercase()).cloned()
    }

    /// The resolved child ids of a block, in document order (unresolved targets dropped).
    pub fn children(&self, id: &BlockId) -> Vec<BlockId> {
        self.blocks
            .get(id)
            .map(|b| {
                b.child_targets()
                    .iter()
                    .filter_map(|t| self.resolve(t))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The resolved reference ids of a block (unresolved targets dropped).
    pub fn references(&self, id: &BlockId) -> Vec<BlockId> {
        self.blocks
            .get(id)
            .map(|b| {
                b.reference_targets()
                    .iter()
                    .filter_map(|t| self.resolve(t))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Blocks that embed (transclude) `id` — incoming child edges.
    pub fn transcluded_by(&self, id: &BlockId) -> Vec<BlockId> {
        let mut out: Vec<BlockId> = self
            .blocks
            .values()
            .filter(|b| self.children(&b.id).contains(id))
            .map(|b| b.id.clone())
            .collect();
        out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        out
    }

    /// Blocks that reference (`[[...]]`) `id` — incoming reference edges.
    pub fn referenced_by(&self, id: &BlockId) -> Vec<BlockId> {
        let mut out: Vec<BlockId> = self
            .blocks
            .values()
            .filter(|b| self.references(&b.id).contains(id))
            .map(|b| b.id.clone())
            .collect();
        out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        out
    }

    /// "Root" blocks: those that nothing transcludes (top-level entries / pages).
    pub fn roots(&self) -> Vec<BlockId> {
        let mut out: Vec<BlockId> = self
            .blocks
            .keys()
            .filter(|id| self.transcluded_by(id).is_empty())
            .cloned()
            .collect();
        out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        out
    }

    /// Build the embed parent-map once: `child id → the blocks that transclude it` (`![[child]]`).
    /// O(blocks·edges); pass the result to [`Vault::lineage_with`] to annotate many blocks without
    /// recomputing the reverse edges per block.
    pub fn embed_parent_map(&self) -> HashMap<BlockId, Vec<BlockId>> {
        let mut map: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
        let mut ids: Vec<&BlockId> = self.blocks.keys().collect();
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        for parent in ids {
            for child in self.children(parent) {
                map.entry(child).or_default().push(parent.clone());
            }
        }
        map
    }

    /// The upward [`Lineage`] of a block: whether it is a root, its minimum embed-depth to a root,
    /// its immediate transcluders, and the distinct root pages reachable upward. Builds the parent
    /// map internally — for many blocks, build it once and call [`Vault::lineage_with`] instead.
    pub fn lineage(&self, id: &BlockId) -> Lineage {
        self.lineage_with(&self.embed_parent_map(), id)
    }

    /// Compute a block's [`Lineage`] using a precomputed `parents` map (see [`embed_parent_map`]).
    /// Walks embed edges **upward** breadth-first, cycle-safe via a visited set, so a DAG with
    /// multiple parents is handled and `depth` is the fewest embed-hops to the nearest root.
    ///
    /// [`embed_parent_map`]: Vault::embed_parent_map
    pub fn lineage_with(&self, parents: &HashMap<BlockId, Vec<BlockId>>, id: &BlockId) -> Lineage {
        let crumb = |bid: &BlockId| Crumb {
            id: bid.clone(),
            title: self
                .block(bid)
                .map(|b| b.display_title())
                .unwrap_or_else(|| bid.as_str().to_string()),
        };
        let immediate = parents.get(id).cloned().unwrap_or_default();
        let is_root = immediate.is_empty();

        let mut visited: HashSet<BlockId> = HashSet::new();
        visited.insert(id.clone());
        let mut queue: VecDeque<(BlockId, usize)> = VecDeque::new();
        queue.push_back((id.clone(), 0));
        let mut root_ids: Vec<BlockId> = Vec::new();
        let mut depth: Option<usize> = None;
        while let Some((node, d)) = queue.pop_front() {
            let ps = parents.get(&node).map(Vec::as_slice).unwrap_or(&[]);
            if ps.is_empty() {
                // No transcluder → this node is a root; record the fewest hops reaching one.
                if !root_ids.contains(&node) {
                    root_ids.push(node.clone());
                }
                depth = Some(depth.map_or(d, |m| m.min(d)));
            } else {
                for p in ps {
                    if visited.insert(p.clone()) {
                        queue.push_back((p.clone(), d + 1));
                    }
                }
            }
        }
        root_ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Lineage {
            is_root,
            depth: depth.unwrap_or(0),
            parents: immediate.iter().map(&crumb).collect(),
            roots: root_ids.iter().map(&crumb).collect(),
        }
    }

    /// Load every block file under `<root>/blocks/` into a vault.
    pub fn from_dir(root: impl AsRef<Path>) -> io::Result<Vault> {
        let mut vault = Vault::new();
        for (id, _abs, source) in read_block_files(root.as_ref())? {
            vault.insert_source(id, &source);
        }
        Ok(vault)
    }

    fn reindex_titles(&mut self) {
        self.by_title.clear();
        // Deterministic: assign titles in id order so collisions resolve predictably.
        let mut ids: Vec<&BlockId> = self.blocks.keys().collect();
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        for id in ids {
            if let Some(t) = self.blocks[id].title.as_ref() {
                let key = t.trim().to_lowercase();
                if !key.is_empty() {
                    self.by_title.entry(key).or_insert_with(|| id.clone());
                }
            }
        }
    }
}

/// The on-disk path of a block file, relative to the vault root.
pub fn block_rel_path(id: &BlockId) -> String {
    format!("{BLOCKS_DIR}/{}.md", id.as_str())
}

/// Validate and normalise a caller-supplied vault-relative path, confining it to the vault.
///
/// Rejects absolute paths and any `..` traversal (and odd components), so a path obtained from
/// an external caller can never escape the vault root via `Path::join`. Returns the cleaned
/// forward-slash relative path. Block writes always go through [`block_rel_path`] (ULID-named,
/// inherently safe); this remains the confinement boundary for any other path input.
pub fn safe_relative_path(rel: &str) -> Result<String, String> {
    let normalized = rel.replace('\\', "/");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return Err("empty path".to_string());
    }
    let p = std::path::Path::new(trimmed);
    if p.is_absolute() || trimmed.starts_with('/') {
        return Err(format!("absolute paths are not allowed: {rel}"));
    }
    let mut clean = Vec::new();
    for comp in trimmed.split('/') {
        match comp {
            "" | "." => continue,
            ".." => return Err(format!("path traversal is not allowed: {rel}")),
            c if c.contains(':') => return Err(format!("invalid path component {c:?} in {rel}")),
            c => clean.push(c),
        }
    }
    if clean.is_empty() {
        return Err(format!("path resolves to the vault root: {rel}"));
    }
    Ok(clean.join("/"))
}

/// Reduce a caller-supplied asset filename to a single safe `stem.ext` component.
///
/// Only the final path component is kept (any directories in `name` are dropped), then characters
/// are restricted to `[A-Za-z0-9._-]` (others become `-`), leading dots/dashes are trimmed, and a
/// single extension is preserved. An empty or extension-only result falls back to `file`. The
/// result is always a safe relative filename — it can never contain a path separator, `..`, or a
/// drive/scheme `:` — so joining it under the vault's assets dir cannot escape the vault.
pub fn sanitize_asset_filename(name: &str) -> String {
    let base = name
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.is_empty() => (s, Some(e)),
        _ => (base.as_str(), None),
    };
    let clean = |s: &str| -> String {
        let mapped: String = s
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        mapped.trim_matches(['.', '-', ' ']).to_string()
    };
    let mut stem = clean(stem);
    if stem.is_empty() {
        stem = "file".to_string();
    }
    match ext.map(clean).filter(|e| !e.is_empty()) {
        Some(ext) => format!("{stem}.{ext}"),
        None => stem,
    }
}

/// Read every `blocks/<ulid>.md` file under `root`, returning `(id, abs path, source)`.
/// Files whose stem is not a valid ULID are skipped (they are not mdkb blocks).
pub fn read_block_files(root: &Path) -> io::Result<Vec<(BlockId, std::path::PathBuf, String)>> {
    let dir = root.join(BLOCKS_DIR);
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let stem = match p.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };
        let id = match BlockId::parse(stem) {
            Ok(id) => id,
            Err(_) => continue, // not a block file
        };
        let source = match std::fs::read_to_string(&p) {
            Ok(s) => s,
            Err(_) => continue, // unreadable / non-UTF-8: skip, never mangle
        };
        out.push((id, p, source));
    }
    out.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> BlockId {
        BlockId::generate()
    }

    #[test]
    fn resolves_by_id_and_title() {
        let mut v = Vault::new();
        let a = id();
        v.insert_source(a.clone(), "---\ntitle: Alpha\n---\nbody\n");
        assert_eq!(v.resolve(a.as_str()), Some(a.clone()));
        assert_eq!(v.resolve("alpha"), Some(a.clone()));
        assert_eq!(v.resolve("ALPHA"), Some(a));
        assert_eq!(v.resolve("missing"), None);
    }

    #[test]
    fn children_and_backlinks_via_embeds() {
        let mut v = Vault::new();
        let child = id();
        let parent = id();
        v.insert_source(child.clone(), "---\ntitle: Child\n---\nchild body\n");
        v.insert_source(parent.clone(), &format!("intro ![[{child}]] end\n"));
        assert_eq!(v.children(&parent), vec![child.clone()]);
        assert_eq!(v.transcluded_by(&child), vec![parent.clone()]);
        // child is not a root (it is transcluded); parent is a root.
        assert!(v.roots().contains(&parent));
        assert!(!v.roots().contains(&child));
    }

    #[test]
    fn lineage_reports_roots_depth_and_multiple_parents() {
        let mut v = Vault::new();
        let leaf = id();
        let mid = id();
        let pagea = id();
        let pageb = id();
        // Graph: pagea embeds mid; mid embeds leaf; pageb ALSO embeds leaf directly.
        v.insert_source(leaf.clone(), "---\ntitle: Leaf\n---\nleaf body\n");
        v.insert_source(mid.clone(), &format!("---\ntitle: Mid\n---\n![[{leaf}]]\n"));
        v.insert_source(
            pagea.clone(),
            &format!("---\ntitle: PageA\n---\n![[{mid}]]\n"),
        );
        v.insert_source(
            pageb.clone(),
            &format!("---\ntitle: PageB\n---\n![[{leaf}]]\n"),
        );

        // A root: is_root, depth 0, roots == itself.
        let la = v.lineage(&pagea);
        assert!(la.is_root);
        assert_eq!(la.depth, 0);
        assert_eq!(
            la.roots.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
            vec![pagea.clone()]
        );

        // The leaf has two parents (mid + PageB) and two reachable roots; nearest root is 1 hop.
        let ll = v.lineage(&leaf);
        assert!(!ll.is_root);
        assert_eq!(
            ll.depth, 1,
            "PageB embeds leaf directly → nearest root is 1 hop"
        );
        let parent_ids: Vec<_> = ll.parents.iter().map(|c| c.id.clone()).collect();
        assert!(parent_ids.contains(&mid) && parent_ids.contains(&pageb));
        let root_ids: Vec<_> = ll.roots.iter().map(|c| c.id.clone()).collect();
        assert!(root_ids.contains(&pagea) && root_ids.contains(&pageb));
        // Crumb titles are resolved.
        assert!(ll.parents.iter().any(|c| c.title == "Mid"));
    }

    #[test]
    fn lineage_is_cycle_safe() {
        // Even if the graph somehow contains an embed cycle, lineage must terminate.
        let mut v = Vault::new();
        let a = id();
        let b = id();
        v.insert_source(a.clone(), &format!("---\ntitle: A\n---\n![[{b}]]\n"));
        v.insert_source(b.clone(), &format!("---\ntitle: B\n---\n![[{a}]]\n"));
        let _ = v.lineage(&a); // must not loop forever
    }

    #[test]
    fn references_are_separate_from_children() {
        let mut v = Vault::new();
        let t = id();
        let s = id();
        v.insert_source(t.clone(), "---\ntitle: Target\n---\nx\n");
        v.insert_source(s.clone(), "see [[Target]] and embed ![[Target]]\n");
        assert_eq!(v.references(&s), vec![t.clone()]);
        assert_eq!(v.children(&s), vec![t.clone()]);
        assert_eq!(v.referenced_by(&t), vec![s.clone()]);
        assert_eq!(v.transcluded_by(&t), vec![s]);
    }

    #[test]
    fn sanitize_asset_filename_keeps_safe_names_and_neutralises_paths() {
        assert_eq!(sanitize_asset_filename("diagram.png"), "diagram.png");
        assert_eq!(sanitize_asset_filename("My Photo.JPG"), "My-Photo.JPG");
        // Directory components and traversal are stripped to a single safe filename.
        assert_eq!(sanitize_asset_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_asset_filename("a/b/c.png"), "c.png");
        assert_eq!(sanitize_asset_filename("C:\\temp\\x.png"), "x.png");
        // No separators, `..`, or scheme `:` can survive.
        for tricky in ["..", "/", ".hidden", "  .png", ""] {
            let out = sanitize_asset_filename(tricky);
            assert!(!out.is_empty());
            assert!(!out.contains('/') && !out.contains('\\') && !out.contains(':'));
            assert_ne!(out, "..");
        }
    }
}
