//! The search index contract (storage-agnostic) plus shared record/query/graph types.
//!
//! Concrete engines (the SQLite + FTS5 impl in `mdkb-index`) implement [`Index`]; everything
//! else programs against the trait so the engine can be swapped (see `AGENTS.md`). Pure
//! ranking, graph, and reachability helpers live here so that logic is shared, not
//! reimplemented per engine. The unit everywhere is the **block** (one file).

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use crate::block::Block;
use crate::id::BlockId;
use crate::vault::Vault;

/// An owned, index-friendly snapshot of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BlockRecord {
    /// Stable block id (its filename stem).
    pub id: BlockId,
    /// Optional human title.
    pub title: Option<String>,
    /// Tag names attached to the block.
    pub tags: Vec<String>,
    /// Fenced-code languages in the block.
    pub langs: Vec<String>,
    /// The Markdown body (verbatim).
    pub content: String,
    /// Title-prepended plain text used for embedding/search context.
    pub contextual_text: String,
    /// Number of resolved children (transclusions).
    pub child_count: usize,
    /// **Human-only** flag (frontmatter `locked: true`). Not persisted in the index (it lives in
    /// the file frontmatter, the source of truth); the service overlays the authoritative value
    /// from the parsed vault when serving a record. Defaults to `false` on the index read path.
    #[cfg_attr(feature = "serde", serde(default))]
    pub locked: bool,
    /// Arbitrary block **properties** — open-ended `key: value` metadata (e.g. `source`,
    /// `verified`, `confidence`). Like `locked`, not persisted as structured rows in the index
    /// (their text is folded into `contextual_text` for full-text search); the service overlays the
    /// authoritative pairs from the parsed vault when serving a record. Empty on the index read path.
    #[cfg_attr(feature = "serde", serde(default))]
    pub props: Vec<(String, String)>,
    /// **Creation time** (RFC 3339 UTC), decoded from the block's ULID id — never stored. The
    /// service fills this in when serving a record. `None` only if the id isn't a decodable ULID.
    #[cfg_attr(feature = "serde", serde(default))]
    pub created: Option<String>,
    /// **Last-modified time** (RFC 3339 UTC) from the block's `updated:` frontmatter, stamped on
    /// each write. Like `locked`/`props`, overlaid from the vault by the service. `None` for a
    /// block not written since timestamps were introduced (absence is normal, never an error).
    #[cfg_attr(feature = "serde", serde(default))]
    pub updated: Option<String>,
}

impl BlockRecord {
    /// Build a record from a parsed block and its resolved child count.
    pub fn from_block(block: &Block, child_count: usize) -> BlockRecord {
        BlockRecord {
            id: block.id.clone(),
            title: block.title.clone(),
            tags: block.tag_names().iter().map(|s| s.to_string()).collect(),
            langs: block.langs.clone(),
            content: block.body.clone(),
            contextual_text: block.contextual_text(),
            child_count,
            locked: block.locked,
            props: block.props.clone(),
            created: block.created(),
            updated: block.updated.clone(),
        }
    }

    /// A short display title for this record.
    pub fn display_title(&self) -> String {
        if let Some(t) = &self.title {
            if !t.trim().is_empty() {
                return t.trim().to_string();
            }
        }
        for line in self.content.lines() {
            let t = line.trim().trim_start_matches('#').trim();
            if !t.is_empty() {
                let t: String = t.chars().take(80).collect();
                return t;
            }
        }
        self.id.to_string()
    }

    /// The value of property `key`, if present (case-insensitive on the key).
    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// The result of a link/embed write — what was actually written. Reports when a requested
/// transclusion was **downgraded** to a plain reference to avoid a cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum LinkOutcome {
    /// A plain `[[reference]]` was written (as requested).
    Reference,
    /// A `![[transclusion]]` was written (as requested).
    Transclusion,
    /// A transclusion was requested but would have formed a cycle, so a plain `[[reference]]`
    /// was written instead.
    DowngradedToReference,
}

/// The relationship a link expresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LinkKind {
    /// `![[...]]` — an embed/transclusion (a child edge).
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

/// A directed edge from one block to another (resolved), or a dangling directive.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LinkRow {
    /// The block that contains the directive.
    pub source_id: BlockId,
    /// Resolved target block id, if the target resolved.
    pub target_id: Option<BlockId>,
    /// The raw target token (ULID or title) as written.
    pub target: String,
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
    /// Query embedding for semantic search; fused with keyword via RRF when both present.
    pub vector: Option<Vec<f32>>,
    /// The embedding model that produced `vector`; only same-model vectors are compared.
    pub vector_model: Option<String>,
    /// Tags that must all be present (AND).
    pub tags: Vec<String>,
    /// Required fenced-code language.
    pub lang: Option<String>,
    /// Keep only blocks **created** on/after this RFC 3339 UTC instant (inclusive).
    pub created_after: Option<String>,
    /// Keep only blocks **created** strictly before this RFC 3339 UTC instant.
    pub created_before: Option<String>,
    /// Keep only blocks **updated** on/after this RFC 3339 UTC instant (inclusive).
    pub updated_after: Option<String>,
    /// Keep only blocks **updated** strictly before this RFC 3339 UTC instant (the staleness
    /// filter). A block with no `updated:` time does not match an updated bound.
    pub updated_before: Option<String>,
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

    /// Parse a free-text query string with inline operators into a structured [`SearchQuery`].
    ///
    /// This is the **single source of truth** for mdkb's search-operator syntax, so every
    /// surface (the desktop search box, the CLI, an MCP raw query) parses it identically rather
    /// than re-implementing it. Recognised operators (case-insensitive keys):
    ///
    /// - `tag:NAME` or `#NAME` — require the tag `NAME` (repeatable; all required, AND).
    /// - `lang:NAME` or `code:NAME` — require a fenced code block in language `NAME`.
    /// - `created:after:DATE` / `created:before:DATE` — bound by creation time.
    /// - `updated:after:DATE` / `updated:before:DATE` — bound by last-modified time. `DATE` is a
    ///   `YYYY-MM-DD` date or a full RFC 3339 timestamp; an unparsable value is ignored.
    ///
    /// Everything else becomes the free-text (FTS) part. A leading `#` is treated as a tag only
    /// when it looks like a tag token (`#word`), so Markdown headings pasted into the box still
    /// search as text. Returns a query whose `text` is `None` when no free text remains.
    pub fn parse(input: &str) -> SearchQuery {
        let mut tags: Vec<String> = Vec::new();
        let mut lang: Option<String> = None;
        let mut q = SearchQuery::default();
        let mut words: Vec<&str> = Vec::new();

        for token in input.split_whitespace() {
            if let Some(rest) = strip_ci_prefix(token, "tag:") {
                push_unique(&mut tags, rest);
            } else if let Some(rest) = strip_ci_prefix(token, "lang:") {
                set_lang(&mut lang, rest);
            } else if let Some(rest) = strip_ci_prefix(token, "code:") {
                set_lang(&mut lang, rest);
            } else if let Some(rest) = strip_ci_prefix(token, "created:after:") {
                q.created_after = crate::clock::parse_query_date(rest);
            } else if let Some(rest) = strip_ci_prefix(token, "created:before:") {
                q.created_before = crate::clock::parse_query_date(rest);
            } else if let Some(rest) = strip_ci_prefix(token, "updated:after:") {
                q.updated_after = crate::clock::parse_query_date(rest);
            } else if let Some(rest) = strip_ci_prefix(token, "updated:before:") {
                q.updated_before = crate::clock::parse_query_date(rest);
            } else if let Some(rest) = token.strip_prefix('#') {
                // `#tag` shorthand — only when it's a real tag token (not a bare `#`).
                if !rest.is_empty() && rest.chars().all(is_tag_char) {
                    push_unique(&mut tags, rest);
                } else {
                    words.push(token);
                }
            } else {
                words.push(token);
            }
        }

        q.text = if words.is_empty() {
            None
        } else {
            Some(words.join(" "))
        };
        q.tags = tags;
        q.lang = lang;
        q
    }

    /// The effective limit (defaulting `0` to 50).
    pub fn effective_limit(&self) -> usize {
        if self.limit == 0 {
            50
        } else {
            self.limit
        }
    }

    /// Whether any created/updated date bound is set.
    pub fn has_date_filter(&self) -> bool {
        self.created_after.is_some()
            || self.created_before.is_some()
            || self.updated_after.is_some()
            || self.updated_before.is_some()
    }

    /// Whether `rec` satisfies the created/updated date bounds. Comparison is lexical on the shared
    /// fixed RFC 3339 format (so it is chronological). A `None` timestamp never satisfies a bound
    /// (an unknown time can't be proven in range), so e.g. a never-updated block is excluded by an
    /// `updated_*` filter rather than wrongly matched.
    pub fn matches_dates(&self, rec: &BlockRecord) -> bool {
        fn lower(bound: &Option<String>, val: &Option<String>) -> bool {
            match bound {
                None => true,
                Some(b) => matches!(val, Some(v) if v.as_str() >= b.as_str()),
            }
        }
        fn upper(bound: &Option<String>, val: &Option<String>) -> bool {
            match bound {
                None => true,
                Some(b) => matches!(val, Some(v) if v.as_str() < b.as_str()),
            }
        }
        lower(&self.created_after, &rec.created)
            && upper(&self.created_before, &rec.created)
            && lower(&self.updated_after, &rec.updated)
            && upper(&self.updated_before, &rec.updated)
    }
}

/// Strip a case-insensitive prefix, returning the remainder when it matched and is non-empty.
fn strip_ci_prefix<'a>(token: &'a str, prefix: &str) -> Option<&'a str> {
    if token.len() >= prefix.len() && token[..prefix.len()].eq_ignore_ascii_case(prefix) {
        let rest = &token[prefix.len()..];
        if rest.is_empty() {
            None
        } else {
            Some(rest)
        }
    } else {
        None
    }
}

fn push_unique(tags: &mut Vec<String>, value: &str) {
    let v = value.to_lowercase();
    if !v.is_empty() && !tags.contains(&v) {
        tags.push(v);
    }
}

fn set_lang(lang: &mut Option<String>, value: &str) {
    if !value.is_empty() {
        *lang = Some(value.to_lowercase());
    }
}

/// Characters allowed in a `#tag` shorthand token.
fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_' || c == '/'
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
    /// Number of indexed blocks.
    pub blocks: usize,
    /// Number of root blocks (not transcluded by anything).
    pub roots: usize,
    /// Number of blocks with a stored embedding.
    pub embedded: usize,
}

/// A tag and how many blocks carry it. Used by the tag browser / discovery surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TagCount {
    /// The tag name.
    pub tag: String,
    /// Number of blocks carrying it.
    pub count: usize,
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

/// The storage-agnostic search index. The unit is the block (one file).
pub trait Index {
    /// Upsert a single block plus its resolved outgoing `links`.
    fn reindex_block(&mut self, record: &BlockRecord, links: &[LinkRow]) -> Result<()>;

    /// Remove a block (and its links) from the index.
    fn remove_block(&mut self, id: &BlockId) -> Result<()>;

    /// Remove everything from the index.
    fn clear(&mut self) -> Result<()>;

    /// Keyword/tag/lang search.
    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>>;

    /// Fetch a single block record by id.
    fn block(&self, id: &BlockId) -> Result<Option<BlockRecord>>;

    /// Outgoing links from a block.
    fn links_from(&self, id: &BlockId) -> Result<Vec<LinkRow>>;

    /// Incoming references/transclusions of a block.
    fn backlinks(&self, id: &BlockId) -> Result<Vec<LinkRow>>;

    /// Index statistics.
    fn stats(&self) -> Result<IndexStats>;

    /// All tags present in the index with the number of blocks carrying each, for tag
    /// discovery. Order is unspecified; callers sort as they wish.
    fn tag_counts(&self) -> Result<Vec<TagCount>>;

    /// Store (or replace) the embedding for a block, tagged with the producing `model_id`.
    fn set_embedding(&mut self, _id: &BlockId, _model_id: &str, _vector: &[f32]) -> Result<()> {
        Ok(())
    }

    /// Whether a block already has a stored embedding.
    fn has_embedding(&self, _id: &BlockId) -> Result<bool> {
        Ok(false)
    }

    /// Rebuild the entire index from a vault (clear + reindex every block).
    fn rebuild(&mut self, vault: &Vault) -> Result<()> {
        self.clear()?;
        for block in vault.blocks() {
            let record = BlockRecord::from_block(block, vault.children(&block.id).len());
            let links = block_links(vault, block);
            self.reindex_block(&record, &links)?;
        }
        Ok(())
    }
}

/// Compute the resolved link rows for a block, given a vault for target resolution.
///
/// Shared so every engine extracts edges identically. Each directive becomes one row;
/// unresolved targets keep `target_id = None` (dangling) but are still recorded so the health
/// view can surface them.
pub fn block_links(vault: &Vault, block: &Block) -> Vec<LinkRow> {
    block
        .references()
        .into_iter()
        .map(|r| LinkRow {
            source_id: block.id.clone(),
            target_id: vault.resolve(&r.target),
            target: r.target,
            kind: if r.embed {
                LinkKind::Transcludes
            } else {
                LinkKind::References
            },
        })
        .collect()
}

// ---------- knowledge graph ----------

/// A node in the knowledge graph (one per block).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GraphNode {
    /// Block id.
    pub id: BlockId,
    /// Display title.
    pub title: String,
    /// Incoming edge weight (how often this block is linked to/transcluded).
    pub in_degree: usize,
    /// Outgoing edge weight.
    pub out_degree: usize,
    /// Whether this block is a root (nothing transcludes it).
    pub root: bool,
}

/// A directed edge between two blocks in the knowledge graph.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GraphEdge {
    /// Source block id.
    pub source: BlockId,
    /// Target block id.
    pub target: BlockId,
    /// Reference vs transclusion.
    pub kind: LinkKind,
}

/// The whole vault rendered as a block-level link graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GraphData {
    /// One node per block.
    pub nodes: Vec<GraphNode>,
    /// Directed edges between blocks (resolved only).
    pub edges: Vec<GraphEdge>,
}

/// Build the block-level knowledge graph from the vault.
pub fn link_graph(vault: &Vault) -> GraphData {
    use std::collections::BTreeMap;

    let mut nodes: BTreeMap<String, GraphNode> = BTreeMap::new();
    for block in vault.blocks() {
        nodes.insert(
            block.id.as_str().to_string(),
            GraphNode {
                id: block.id.clone(),
                title: block.display_title(),
                in_degree: 0,
                out_degree: 0,
                root: false,
            },
        );
    }

    let mut edges = Vec::new();
    for block in vault.blocks() {
        for row in block_links(vault, block) {
            let Some(target) = row.target_id else {
                continue;
            };
            if target == block.id {
                continue;
            }
            if let Some(n) = nodes.get_mut(block.id.as_str()) {
                n.out_degree += 1;
            }
            if let Some(n) = nodes.get_mut(target.as_str()) {
                n.in_degree += 1;
            }
            edges.push(GraphEdge {
                source: block.id.clone(),
                target,
                kind: row.kind,
            });
        }
    }
    for id in vault.roots() {
        if let Some(n) = nodes.get_mut(id.as_str()) {
            n.root = true;
        }
    }

    GraphData {
        nodes: nodes.into_values().collect(),
        edges,
    }
}

/// Does `start` reach `goal` by following **transclusion** edges (transitively)?
///
/// This is the reachability test behind cycle *prevention*: creating an embed
/// `source ![[target]]` would close a cycle iff `target` already transcludes its way back to
/// `source`. References (`[[...]]`) are intentionally ignored — only embeds expand at render
/// time, so only embeds can loop. A block trivially reaches itself.
pub fn transclusion_reaches(vault: &Vault, start: &BlockId, goal: &BlockId) -> bool {
    let mut stack = vec![start.clone()];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if &id == goal {
            return true;
        }
        if !seen.insert(id.clone()) {
            continue;
        }
        stack.extend(vault.children(&id));
    }
    false
}

/// Fuse keyword and vector rankings into one ordering via Reciprocal Rank Fusion (RRF).
///
/// `keyword` and `vector` are each `(BlockId, _)` lists already sorted best-first. RRF is
/// robust to incomparable score scales (FTS rank vs cosine distance). Returns ids sorted by
/// fused score desc.
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

/// In-memory [`Index`] for tests across the workspace.
#[doc(hidden)]
pub mod testing {
    use super::*;
    use std::collections::HashMap;

    /// A simple in-memory index: enough for unit tests, no FTS/vectors.
    #[derive(Default)]
    pub struct MemIndex {
        blocks: HashMap<BlockId, BlockRecord>,
        links: HashMap<BlockId, Vec<LinkRow>>,
        embeddings: HashMap<BlockId, (String, Vec<f32>)>,
    }

    impl Index for MemIndex {
        fn reindex_block(&mut self, record: &BlockRecord, links: &[LinkRow]) -> Result<()> {
            self.blocks.insert(record.id.clone(), record.clone());
            self.links.insert(record.id.clone(), links.to_vec());
            Ok(())
        }

        fn remove_block(&mut self, id: &BlockId) -> Result<()> {
            self.blocks.remove(id);
            self.links.remove(id);
            self.embeddings.remove(id);
            Ok(())
        }

        fn clear(&mut self) -> Result<()> {
            self.blocks.clear();
            self.links.clear();
            self.embeddings.clear();
            Ok(())
        }

        fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>> {
            let needle = query.text.as_deref().unwrap_or("").to_lowercase();
            let mut hits: Vec<SearchHit> = self
                .blocks
                .values()
                .filter(|b| {
                    (needle.is_empty() || b.content.to_lowercase().contains(&needle))
                        && query
                            .tags
                            .iter()
                            .all(|t| b.tags.iter().any(|x| x.eq_ignore_ascii_case(t)))
                        && query
                            .lang
                            .as_ref()
                            .map(|l| b.langs.iter().any(|x| x.eq_ignore_ascii_case(l)))
                            .unwrap_or(true)
                })
                .map(|b| SearchHit {
                    block: b.clone(),
                    score: 1.0,
                })
                .collect();
            hits.sort_by(|a, b| a.block.id.as_str().cmp(b.block.id.as_str()));
            hits.truncate(query.effective_limit());
            Ok(hits)
        }

        fn block(&self, id: &BlockId) -> Result<Option<BlockRecord>> {
            Ok(self.blocks.get(id).cloned())
        }

        fn links_from(&self, id: &BlockId) -> Result<Vec<LinkRow>> {
            Ok(self.links.get(id).cloned().unwrap_or_default())
        }

        fn backlinks(&self, id: &BlockId) -> Result<Vec<LinkRow>> {
            let mut out = Vec::new();
            for rows in self.links.values() {
                for r in rows {
                    if r.target_id.as_ref() == Some(id) {
                        out.push(r.clone());
                    }
                }
            }
            Ok(out)
        }

        fn stats(&self) -> Result<IndexStats> {
            Ok(IndexStats {
                blocks: self.blocks.len(),
                roots: 0,
                embedded: self.embeddings.len(),
            })
        }

        fn tag_counts(&self) -> Result<Vec<TagCount>> {
            let mut counts: HashMap<String, usize> = HashMap::new();
            for b in self.blocks.values() {
                for t in &b.tags {
                    *counts.entry(t.clone()).or_insert(0) += 1;
                }
            }
            let mut out: Vec<TagCount> = counts
                .into_iter()
                .map(|(tag, count)| TagCount { tag, count })
                .collect();
            out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tag.cmp(&b.tag)));
            Ok(out)
        }

        fn set_embedding(&mut self, id: &BlockId, model_id: &str, vector: &[f32]) -> Result<()> {
            self.embeddings
                .insert(id.clone(), (model_id.to_string(), vector.to_vec()));
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

    fn vault_with(blocks: &[(&str, &str)]) -> (Vault, Vec<BlockId>) {
        let mut v = Vault::new();
        let mut ids = Vec::new();
        for (_, src) in blocks {
            let id = BlockId::generate();
            v.insert_source(id.clone(), src);
            ids.push(id);
        }
        (v, ids)
    }

    #[test]
    fn block_links_resolve_and_classify() {
        let mut v = Vault::new();
        let target = BlockId::generate();
        let src = BlockId::generate();
        v.insert_source(target.clone(), "---\ntitle: T\n---\nx\n");
        v.insert_source(
            src.clone(),
            &format!("ref [[{target}]] embed ![[{target}]]\n"),
        );
        let rows = block_links(&v, v.block(&src).unwrap());
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .any(|r| r.kind == LinkKind::References && r.target_id == Some(target.clone())));
        assert!(rows
            .iter()
            .any(|r| r.kind == LinkKind::Transcludes && r.target_id == Some(target.clone())));
    }

    #[test]
    fn dangling_target_is_recorded() {
        let (v, ids) = vault_with(&[("a", "see [[ghost-block]]\n")]);
        let rows = block_links(&v, v.block(&ids[0]).unwrap());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target_id, None);
        assert_eq!(rows[0].target, "ghost-block");
    }

    #[test]
    fn graph_counts_degrees_and_roots() {
        let mut v = Vault::new();
        let child = BlockId::generate();
        let parent = BlockId::generate();
        v.insert_source(child.clone(), "---\ntitle: Child\n---\nx\n");
        v.insert_source(parent.clone(), &format!("![[{child}]]\n"));
        let g = link_graph(&v);
        assert_eq!(g.nodes.len(), 2);
        let cn = g.nodes.iter().find(|n| n.id == child).unwrap();
        assert_eq!(cn.in_degree, 1);
        assert!(!cn.root);
        let pn = g.nodes.iter().find(|n| n.id == parent).unwrap();
        assert!(pn.root);
        assert_eq!(pn.out_degree, 1);
    }

    #[test]
    fn transclusion_reaches_follows_only_embeds() {
        let mut v = Vault::new();
        let a = BlockId::generate();
        let b = BlockId::generate();
        v.insert_source(a.clone(), &format!("![[{b}]]\n"));
        v.insert_source(b.clone(), "leaf\n");
        assert!(transclusion_reaches(&v, &a, &b));
        assert!(!transclusion_reaches(&v, &b, &a));
        // A reference does NOT count as reachability.
        v.insert_source(b.clone(), &format!("see [[{a}]]\n"));
        assert!(!transclusion_reaches(&v, &b, &a));
    }

    #[test]
    fn rrf_rewards_agreement() {
        let a = BlockId::generate();
        let b = BlockId::generate();
        let c = BlockId::generate();
        let keyword = vec![a.clone(), b.clone()];
        let vector = vec![a.clone(), c.clone()];
        let fused = reciprocal_rank_fusion(&keyword, &vector, 60.0);
        assert_eq!(fused[0].0, a);
        assert!(fused[0].1 > fused[1].1);
    }

    #[test]
    fn parse_plain_text_only() {
        let q = SearchQuery::parse("ingress timeout");
        assert_eq!(q.text.as_deref(), Some("ingress timeout"));
        assert!(q.tags.is_empty());
        assert_eq!(q.lang, None);
    }

    #[test]
    fn parse_extracts_tag_lang_and_text() {
        let q = SearchQuery::parse("tag:k8s lang:Kusto cluster query");
        assert_eq!(q.text.as_deref(), Some("cluster query"));
        assert_eq!(q.tags, vec!["k8s"]);
        // lang is normalised to lowercase.
        assert_eq!(q.lang.as_deref(), Some("kusto"));
    }

    #[test]
    fn parse_hash_shorthand_and_code_alias() {
        let q = SearchQuery::parse("#networking code:rust retry");
        assert_eq!(q.tags, vec!["networking"]);
        assert_eq!(q.lang.as_deref(), Some("rust"));
        assert_eq!(q.text.as_deref(), Some("retry"));
    }

    #[test]
    fn parse_dedupes_tags_and_handles_no_free_text() {
        let q = SearchQuery::parse("tag:k8s #K8S tag:net");
        assert_eq!(q.tags, vec!["k8s", "net"]);
        assert_eq!(q.text, None);
    }

    #[test]
    fn parse_bare_hash_and_empty_operator_stay_text() {
        // A lone `#` or `tag:` with no value is not an operator — keep it as text.
        let q = SearchQuery::parse("# tag: heading");
        assert_eq!(q.tags, Vec::<String>::new());
        assert_eq!(q.lang, None);
        assert_eq!(q.text.as_deref(), Some("# tag: heading"));
    }
}
