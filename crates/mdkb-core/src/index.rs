//! The search index contract.
//!
//! This module defines the **storage-agnostic** index interface plus the owned record,
//! query, and hit types. Concrete engines (e.g. the SQLite + FTS5 + sqlite-vec impl in the
//! `mdkb-index` crate) implement [`Index`]; everything else programs against the trait so
//! the engine can be swapped without touching callers (see `AGENTS.md`).
//!
//! Pure ranking helpers (hybrid fusion of keyword + vector scores) live here so that logic
//! is shared, not reimplemented per engine.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use crate::block::Block;
use crate::id::BlockId;
use crate::link::{extract_references, Anchor};
use crate::vault::{Page, Vault};

/// An owned, index-friendly snapshot of a block and the page it lives on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BlockRecord {
    /// Stable block id.
    pub id: BlockId,
    /// Vault-relative page path.
    pub page_path: String,
    /// Structural kind (see [`crate::block::BlockKind::kind_str`]).
    pub kind: String,
    /// Heading level, if applicable.
    pub heading_level: Option<u8>,
    /// Fence language, if a code block.
    pub lang: Option<String>,
    /// Heading lineage breadcrumb.
    pub lineage: Vec<String>,
    /// Tag names attached to the block.
    pub tags: Vec<String>,
    /// Raw block content.
    pub content: String,
    /// Lineage-prepended text used for embedding/search context.
    pub contextual_text: String,
}

impl BlockRecord {
    /// Build a record from a page path and a parsed block.
    pub fn from_block(page_path: impl Into<String>, block: &Block) -> BlockRecord {
        BlockRecord {
            id: block.id.clone(),
            page_path: page_path.into(),
            kind: block.kind.kind_str().to_string(),
            heading_level: block.kind.heading_level(),
            lang: block.lang.clone(),
            lineage: block.lineage.clone(),
            tags: block.tag_names().iter().map(|s| s.to_string()).collect(),
            content: block.content.clone(),
            contextual_text: block.contextual_text(),
        }
    }
}

/// The relationship a link expresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LinkKind {
    /// `![[...]]` — an embed/transclusion.
    Transcludes,
    /// `[[...]]` — a plain reference.
    References,
}

impl LinkKind {
    /// Short stable string form.
    pub fn as_str(&self) -> &'static str {
        match self {
            LinkKind::Transcludes => "transcludes",
            LinkKind::References => "references",
        }
    }
}

/// A directed edge from one block to a target (resolved page path + optional block id).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LinkRow {
    /// The block that contains the reference.
    pub source_id: BlockId,
    /// Resolved target page path, if the page resolved.
    pub target_page: Option<String>,
    /// Target block id, if the anchor was an id (or resolved to one).
    pub target_id: Option<BlockId>,
    /// Raw anchor text (heading text or id), if any.
    pub target_anchor: Option<String>,
    /// Embed vs plain reference.
    pub kind: LinkKind,
}

/// A keyword/tag/lang search request.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct SearchQuery {
    /// Full-text query (FTS). `None` matches everything (subject to filters).
    pub text: Option<String>,
    /// Query embedding for semantic search. When present alongside `text`, results are
    /// fused (keyword + vector) via reciprocal rank fusion.
    pub vector: Option<Vec<f32>>,
    /// The embedding model that produced `vector`. When set, only stored vectors from the
    /// same model are compared, so different embedding spaces (e.g. after an ONNX→hash
    /// fallback) are never mixed into one ranking.
    pub vector_model: Option<String>,
    /// Tags that must all be present (AND).
    pub tags: Vec<String>,
    /// Required fence language (e.g. `kusto`).
    pub lang: Option<String>,
    /// Restrict to a single page path.
    pub page: Option<String>,
    /// Max results. `0` is treated as the default (50).
    pub limit: usize,
}

impl SearchQuery {
    /// A query for plain text.
    pub fn text(q: impl Into<String>) -> SearchQuery {
        SearchQuery {
            text: Some(q.into()),
            ..Default::default()
        }
    }

    /// The effective limit (defaulting `0` to 50).
    pub fn effective_limit(&self) -> usize {
        if self.limit == 0 {
            50
        } else {
            self.limit
        }
    }
}

/// A single search result.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SearchHit {
    /// The matching block.
    pub block: BlockRecord,
    /// Combined relevance score (higher is better).
    pub score: f64,
}

/// Lightweight index statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IndexStats {
    /// Number of indexed pages.
    pub pages: usize,
    /// Number of indexed blocks.
    pub blocks: usize,
    /// Number of blocks with a stored embedding.
    pub embedded: usize,
}

/// An error from an index operation.
#[derive(Debug)]
pub struct IndexError(pub String);

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "index error: {}", self.0)
    }
}

impl Error for IndexError {}

impl IndexError {
    /// Wrap any displayable error.
    pub fn new(e: impl fmt::Display) -> IndexError {
        IndexError(e.to_string())
    }
}

/// Convenient result alias.
pub type Result<T> = std::result::Result<T, IndexError>;

/// The storage-agnostic search index.
pub trait Index {
    /// Upsert every block of a page (plus its resolved outgoing `links`), removing any
    /// blocks/links that no longer exist on it. Link rows are computed by the caller via
    /// [`page_links`] so target resolution stays shared in core.
    fn reindex_page(&mut self, page: &Page, links: &[LinkRow]) -> Result<()>;

    /// Remove a page and all its blocks from the index.
    fn remove_page(&mut self, page_path: &str) -> Result<()>;

    /// Remove everything from the index.
    fn clear(&mut self) -> Result<()>;

    /// Keyword/tag/lang search.
    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>>;

    /// Fetch a single block record by id.
    fn block(&self, id: &BlockId) -> Result<Option<BlockRecord>>;

    /// Outgoing links from a block.
    fn links_from(&self, id: &BlockId) -> Result<Vec<LinkRow>>;

    /// Incoming references: blocks that link to (or transclude) the given block id.
    fn backlinks(&self, id: &BlockId) -> Result<Vec<LinkRow>>;

    /// Index statistics.
    fn stats(&self) -> Result<IndexStats>;

    /// Store (or replace) the embedding for a block, tagged with the `model_id` that
    /// produced it. Default: a no-op for engines without vector support.
    fn set_embedding(&mut self, _id: &BlockId, _model_id: &str, _vector: &[f32]) -> Result<()> {
        Ok(())
    }

    /// Whether a block already has a stored embedding. Default: `false`.
    fn has_embedding(&self, _id: &BlockId) -> Result<bool> {
        Ok(false)
    }

    /// Rebuild the entire index from a vault (clear + reindex every page).
    fn rebuild(&mut self, vault: &Vault) -> Result<()> {
        self.clear()?;
        for page in vault.pages() {
            let links = page_links(vault, page);
            self.reindex_page(page, &links)?;
        }
        Ok(())
    }
}

/// Compute the resolved link rows for a page, given a vault for target resolution.
///
/// Shared so every engine extracts edges identically. Heading anchors are resolved to a
/// concrete block id when possible.
pub fn page_links(vault: &Vault, page: &Page) -> Vec<LinkRow> {
    let mut rows = Vec::new();
    for block in &page.doc.blocks {
        for r in extract_references(&block.content) {
            let target_page_name = r.target.page.clone();
            let resolved_page = match &target_page_name {
                Some(name) => vault.page(name),
                None => Some(page),
            };
            let (target_page, target_id, target_anchor) = match (&r.target.anchor, resolved_page) {
                (Some(Anchor::Id(id)), rp) => (
                    rp.map(|p| p.path.clone()),
                    Some(id.clone()),
                    Some(id.to_string()),
                ),
                (Some(Anchor::Heading(h)), Some(rp)) => {
                    let id = rp
                        .doc
                        .blocks
                        .iter()
                        .find(|b| {
                            b.kind.heading_level().is_some()
                                && heading_label(&b.content).eq_ignore_ascii_case(h.trim())
                        })
                        .map(|b| b.id.clone());
                    (Some(rp.path.clone()), id, Some(h.clone()))
                }
                (Some(Anchor::Heading(h)), None) => (None, None, Some(h.clone())),
                (None, rp) => (rp.map(|p| p.path.clone()), None, None),
            };
            rows.push(LinkRow {
                source_id: block.id.clone(),
                target_page,
                target_id,
                target_anchor,
                kind: if r.embed {
                    LinkKind::Transcludes
                } else {
                    LinkKind::References
                },
            });
        }
    }
    rows
}

fn heading_label(content: &str) -> String {
    content
        .trim_start()
        .trim_start_matches('#')
        .trim()
        .trim_end_matches('#')
        .trim()
        .to_string()
}

/// Fuse keyword and vector scores into a single ranking via Reciprocal Rank Fusion (RRF).
///
/// `keyword` and `vector` are each `(BlockId, rank_score)` lists already sorted best-first.
/// RRF is robust to incomparable score scales (FTS rank vs cosine distance), which is why
/// it is used instead of naive score addition. Returns ids sorted by fused score desc.
pub fn reciprocal_rank_fusion(
    keyword: &[BlockId],
    vector: &[BlockId],
    k: f64,
) -> Vec<(BlockId, f64)> {
    let mut scores: HashMap<BlockId, f64> = HashMap::new();
    for (rank, id) in keyword.iter().enumerate() {
        *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + (rank as f64) + 1.0);
    }
    for (rank, id) in vector.iter().enumerate() {
        *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + (rank as f64) + 1.0);
    }
    let mut fused: Vec<(BlockId, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused
}

/// A minimal in-memory [`Index`] for tests across the crate (no storage backend).
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::collections::HashMap;

    /// In-memory index: enough to exercise sync/service logic without SQLite.
    #[derive(Default)]
    pub(crate) struct MemIndex {
        blocks: HashMap<BlockId, BlockRecord>,
        page_blocks: HashMap<String, Vec<BlockId>>,
        links: Vec<LinkRow>,
        embeddings: HashMap<BlockId, Vec<f32>>,
    }

    impl Index for MemIndex {
        fn reindex_page(&mut self, page: &Page, links: &[LinkRow]) -> Result<()> {
            self.remove_page(&page.path)?;
            let mut ids = Vec::new();
            for b in &page.doc.blocks {
                ids.push(b.id.clone());
                self.blocks
                    .insert(b.id.clone(), BlockRecord::from_block(&page.path, b));
            }
            self.page_blocks.insert(page.path.clone(), ids);
            self.links.extend_from_slice(links);
            Ok(())
        }
        fn remove_page(&mut self, page_path: &str) -> Result<()> {
            if let Some(ids) = self.page_blocks.remove(page_path) {
                for id in &ids {
                    self.blocks.remove(id);
                    self.embeddings.remove(id);
                }
                self.links.retain(|l| !ids.contains(&l.source_id));
            }
            Ok(())
        }
        fn clear(&mut self) -> Result<()> {
            self.blocks.clear();
            self.page_blocks.clear();
            self.links.clear();
            self.embeddings.clear();
            Ok(())
        }
        fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
            // Substring keyword match; enough for tests.
            let mut hits: Vec<SearchHit> = self
                .blocks
                .values()
                .filter(|b| match &query.text {
                    Some(t) => b.content.to_lowercase().contains(&t.to_lowercase()),
                    None => true,
                })
                .filter(|b| {
                    query
                        .lang
                        .as_ref()
                        .is_none_or(|l| b.lang.as_deref() == Some(l))
                })
                .filter(|b| query.tags.iter().all(|t| b.tags.contains(t)))
                .map(|b| SearchHit {
                    block: b.clone(),
                    score: 1.0,
                })
                .collect();
            hits.truncate(query.effective_limit());
            Ok(hits)
        }
        fn block(&self, id: &BlockId) -> Result<Option<BlockRecord>> {
            Ok(self.blocks.get(id).cloned())
        }
        fn links_from(&self, id: &BlockId) -> Result<Vec<LinkRow>> {
            Ok(self
                .links
                .iter()
                .filter(|l| &l.source_id == id)
                .cloned()
                .collect())
        }
        fn backlinks(&self, id: &BlockId) -> Result<Vec<LinkRow>> {
            Ok(self
                .links
                .iter()
                .filter(|l| l.target_id.as_ref() == Some(id))
                .cloned()
                .collect())
        }
        fn stats(&self) -> Result<IndexStats> {
            Ok(IndexStats {
                pages: self.page_blocks.len(),
                blocks: self.blocks.len(),
                embedded: self.embeddings.len(),
            })
        }
        fn set_embedding(&mut self, id: &BlockId, _model_id: &str, vector: &[f32]) -> Result<()> {
            self.embeddings.insert(id.clone(), vector.to_vec());
            Ok(())
        }
        fn has_embedding(&self, id: &BlockId) -> Result<bool> {
            Ok(self.embeddings.contains_key(id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_record_carries_context() {
        let mut v = Vault::new();
        v.insert("a.md", "# H1\n\n## H2\n\nbody #tag here\n");
        v.assign_ids();
        let page = v.page("a").unwrap();
        let body = page
            .doc
            .blocks
            .iter()
            .find(|b| b.content.contains("body"))
            .unwrap();
        let rec = BlockRecord::from_block(&page.path, body);
        assert_eq!(rec.page_path, "a.md");
        assert_eq!(rec.kind, "paragraph");
        assert_eq!(rec.lineage, vec!["H1".to_string(), "H2".to_string()]);
        assert!(rec.tags.contains(&"tag".to_string()));
        assert!(rec.contextual_text.starts_with("H1 > H2"));
    }

    #[test]
    fn page_links_resolves_targets() {
        let mut v = Vault::new();
        v.insert("src.md", "# Kusto Basics\n\nquery body\n");
        v.assign_ids();
        let qid = v
            .page("src")
            .unwrap()
            .doc
            .blocks
            .iter()
            .find(|b| b.content.contains("query body"))
            .unwrap()
            .id
            .clone();
        v.insert(
            "dst.md",
            format!("embed ![[src#{qid}]] and link [[src#Kusto Basics]]\n"),
        );
        v.assign_ids();
        let dst = v.page("dst").unwrap();
        let rows = page_links(&v, dst);
        assert_eq!(rows.len(), 2);
        let embed = rows
            .iter()
            .find(|r| r.kind == LinkKind::Transcludes)
            .unwrap();
        assert_eq!(embed.target_page.as_deref(), Some("src.md"));
        assert_eq!(embed.target_id.as_ref(), Some(&qid));
        let link = rows
            .iter()
            .find(|r| r.kind == LinkKind::References)
            .unwrap();
        // Heading anchor resolves to the heading block's id.
        assert!(link.target_id.is_some());
    }

    #[test]
    fn rrf_rewards_agreement() {
        let a = BlockId::generate();
        let b = BlockId::generate();
        let c = BlockId::generate();
        // `a` ranks well in both lists; `b` only in keyword; `c` only in vector.
        let keyword = vec![a.clone(), b.clone()];
        let vector = vec![a.clone(), c.clone()];
        let fused = reciprocal_rank_fusion(&keyword, &vector, 60.0);
        assert_eq!(fused[0].0, a, "block in both lists should rank first");
        assert!(fused[0].1 > fused[1].1);
    }
}
