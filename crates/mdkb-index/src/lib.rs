//! SQLite implementation of [`mdkb_core::Index`].
//!
//! Uses a bundled SQLite (so there is no system dependency) with an FTS5 virtual table for
//! keyword search. Vector search (sqlite-vec) is layered on in a later phase behind the
//! same trait. The index is a **rebuildable cache** of the Markdown vault: it can be thrown
//! away and reconstructed from the files at any time, so it is never the source of truth.

use std::path::Path;

use mdkb_core::{
    BlockId, BlockRecord, Index, IndexError, IndexStats, LinkKind, LinkRow, Page, SearchHit,
    SearchQuery,
};
use rusqlite::{params, params_from_iter, Connection};

const LINEAGE_SEP: char = '\u{1f}';

/// A SQLite-backed search index.
pub struct SqliteIndex {
    conn: Connection,
}

fn err(e: impl std::fmt::Display) -> IndexError {
    IndexError::new(e)
}

impl SqliteIndex {
    /// Open (creating if needed) an index at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, IndexError> {
        let conn = Connection::open(path).map_err(err)?;
        Self::init(conn)
    }

    /// Open a transient in-memory index (used in tests).
    pub fn open_in_memory() -> Result<Self, IndexError> {
        let conn = Connection::open_in_memory().map_err(err)?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self, IndexError> {
        conn.execute_batch(SCHEMA).map_err(err)?;
        Ok(SqliteIndex { conn })
    }

    /// Borrow the underlying connection (for the vector-store layer and tests).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    fn delete_page_rows(tx: &Connection, page_path: &str) -> Result<(), IndexError> {
        // Remove FTS rows (by rowid) before deleting the backing blocks.
        let mut stmt = tx
            .prepare("SELECT rowid FROM blocks WHERE page_path = ?1")
            .map_err(err)?;
        let rowids: Vec<i64> = stmt
            .query_map(params![page_path], |r| r.get::<_, i64>(0))
            .map_err(err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(err)?;
        for rid in rowids {
            tx.execute("DELETE FROM block_fts WHERE rowid = ?1", params![rid])
                .map_err(err)?;
        }
        tx.execute(
            "DELETE FROM block_tags WHERE block_id IN (SELECT id FROM blocks WHERE page_path = ?1)",
            params![page_path],
        )
        .map_err(err)?;
        tx.execute(
            "DELETE FROM links WHERE source_id IN (SELECT id FROM blocks WHERE page_path = ?1)",
            params![page_path],
        )
        .map_err(err)?;
        tx.execute(
            "DELETE FROM block_vectors WHERE block_id IN (SELECT id FROM blocks WHERE page_path = ?1)",
            params![page_path],
        )
        .map_err(err)?;
        tx.execute(
            "DELETE FROM blocks WHERE page_path = ?1",
            params![page_path],
        )
        .map_err(err)?;
        tx.execute("DELETE FROM pages WHERE path = ?1", params![page_path])
            .map_err(err)?;
        Ok(())
    }

    /// Append the lang/page/tag filter clauses shared by every search path.
    fn push_filters(
        sql: &mut String,
        args: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
        query: &SearchQuery,
    ) {
        if let Some(lang) = &query.lang {
            args.push(Box::new(lang.clone()));
            sql.push_str(&format!(" AND b.lang = ?{}", args.len()));
        }
        if let Some(page) = &query.page {
            args.push(Box::new(page.clone()));
            sql.push_str(&format!(" AND b.page_path = ?{}", args.len()));
        }
        for tag in &query.tags {
            args.push(Box::new(tag.clone()));
            sql.push_str(&format!(
                " AND b.id IN (SELECT block_id FROM block_tags WHERE tag = ?{})",
                args.len()
            ));
        }
    }

    /// FTS5 keyword search with filters, ordered by bm25 relevance.
    fn keyword_hits(
        &self,
        query: &SearchQuery,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        let text = match &query.text {
            Some(t) => t,
            None => return Ok(Vec::new()),
        };
        let match_expr = to_fts_query(text);
        if match_expr.is_empty() {
            return Ok(Vec::new());
        }
        let mut sql = String::from(
            "SELECT b.id, b.page_path, b.kind, b.heading_level, b.lang, b.lineage, b.content, b.contextual_text, b.tags_text, \
             bm25(block_fts) AS rank \
             FROM block_fts JOIN blocks b ON b.rowid = block_fts.rowid \
             WHERE block_fts MATCH ?1",
        );
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(match_expr)];
        Self::push_filters(&mut sql, &mut args, query);
        args.push(Box::new(limit as i64));
        sql.push_str(&format!(" ORDER BY rank LIMIT ?{}", args.len()));

        let mut stmt = self.conn.prepare(&sql).map_err(err)?;
        let rows = stmt
            .query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |r| {
                let rank: f64 = r.get(9)?;
                Ok(SearchHit {
                    block: row_to_record(r)?,
                    score: -rank,
                })
            })
            .map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }

    /// Brute-force cosine vector search with filters.
    ///
    /// Candidate vectors (after applying filters) are scored in Rust. This is simple and
    /// fast enough for a personal KB; a `sqlite-vec` ANN index can replace it behind this
    /// same method if the corpus grows large.
    fn vector_hits(
        &self,
        query: &SearchQuery,
        vector: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        let mut sql = String::from(
            "SELECT b.id, b.page_path, b.kind, b.heading_level, b.lang, b.lineage, b.content, b.contextual_text, b.tags_text, \
             v.embedding \
             FROM blocks b JOIN block_vectors v ON v.block_id = b.id WHERE 1=1",
        );
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        // Only compare vectors produced by the same embedding model, so different spaces
        // (e.g. after an ONNX→hash fallback) are never mixed into one ranking.
        if let Some(model) = &query.vector_model {
            args.push(Box::new(model.clone()));
            sql.push_str(&format!(" AND v.model_id = ?{}", args.len()));
        }
        Self::push_filters(&mut sql, &mut args, query);

        let mut stmt = self.conn.prepare(&sql).map_err(err)?;
        let rows = stmt
            .query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |r| {
                let blob: Vec<u8> = r.get(9)?;
                Ok((row_to_record(r)?, blob))
            })
            .map_err(err)?;

        let mut scored: Vec<SearchHit> = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)?
            .into_iter()
            .map(|(block, blob)| {
                let candidate = mdkb_core::bytes_to_vector(&blob);
                let score = mdkb_core::cosine_similarity(vector, &candidate) as f64;
                SearchHit { block, score }
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        Ok(scored)
    }

    /// Filter-only listing (no text, no vector): blocks in document order.
    fn filter_only_hits(
        &self,
        query: &SearchQuery,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        let mut sql = String::from(
            "SELECT b.id, b.page_path, b.kind, b.heading_level, b.lang, b.lineage, b.content, b.contextual_text, b.tags_text, \
             0.0 FROM blocks b WHERE 1=1",
        );
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        Self::push_filters(&mut sql, &mut args, query);
        args.push(Box::new(limit as i64));
        sql.push_str(&format!(
            " ORDER BY b.page_path, b.ord LIMIT ?{}",
            args.len()
        ));

        let mut stmt = self.conn.prepare(&sql).map_err(err)?;
        let rows = stmt
            .query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |r| {
                Ok(SearchHit {
                    block: row_to_record(r)?,
                    score: 0.0,
                })
            })
            .map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }
}

impl Index for SqliteIndex {
    fn reindex_page(&mut self, page: &Page, links: &[LinkRow]) -> Result<(), IndexError> {
        let tx = self.conn.transaction().map_err(err)?;
        Self::delete_page_rows(&tx, &page.path)?;
        tx.execute("INSERT INTO pages(path) VALUES (?1)", params![page.path])
            .map_err(err)?;

        for (ord, block) in page.doc.blocks.iter().enumerate() {
            let rec = BlockRecord::from_block(&page.path, block);
            let lineage = rec.lineage.join(&LINEAGE_SEP.to_string());
            let tags_text = rec.tags.join(" ");
            tx.execute(
                "INSERT INTO blocks(id, page_path, kind, heading_level, lang, lineage, content, contextual_text, tags_text, ord)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    rec.id.as_str(),
                    rec.page_path,
                    rec.kind,
                    rec.heading_level,
                    rec.lang,
                    lineage,
                    rec.content,
                    rec.contextual_text,
                    tags_text,
                    ord as i64,
                ],
            )
            .map_err(err)?;
            let rowid = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO block_fts(rowid, content, lineage, tags) VALUES (?1,?2,?3,?4)",
                params![
                    rowid,
                    rec.content,
                    lineage.replace(LINEAGE_SEP, " "),
                    tags_text
                ],
            )
            .map_err(err)?;
            for tag in &rec.tags {
                tx.execute(
                    "INSERT INTO block_tags(block_id, tag) VALUES (?1,?2)",
                    params![rec.id.as_str(), tag],
                )
                .map_err(err)?;
            }
        }

        for link in links {
            tx.execute(
                "INSERT INTO links(source_id, target_page, target_id, target_anchor, kind)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    link.source_id.as_str(),
                    link.target_page,
                    link.target_id.as_ref().map(|i| i.as_str().to_string()),
                    link.target_anchor,
                    link.kind.as_str(),
                ],
            )
            .map_err(err)?;
        }

        tx.commit().map_err(err)?;
        Ok(())
    }

    fn remove_page(&mut self, page_path: &str) -> Result<(), IndexError> {
        let tx = self.conn.transaction().map_err(err)?;
        Self::delete_page_rows(&tx, page_path)?;
        tx.commit().map_err(err)?;
        Ok(())
    }

    fn clear(&mut self) -> Result<(), IndexError> {
        self.conn
            .execute_batch(
                "DELETE FROM block_fts; DELETE FROM block_tags; DELETE FROM links; DELETE FROM blocks; DELETE FROM pages; DELETE FROM block_vectors;",
            )
            .map_err(err)
    }

    fn set_embedding(
        &mut self,
        id: &BlockId,
        model_id: &str,
        vector: &[f32],
    ) -> Result<(), IndexError> {
        self.conn
            .execute(
                "INSERT INTO block_vectors(block_id, model_id, dim, embedding) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(block_id) DO UPDATE SET model_id = excluded.model_id, dim = excluded.dim, embedding = excluded.embedding",
                params![id.as_str(), model_id, vector.len() as i64, mdkb_core::vector_to_bytes(vector)],
            )
            .map_err(err)?;
        Ok(())
    }

    fn has_embedding(&self, id: &BlockId) -> Result<bool, IndexError> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM block_vectors WHERE block_id = ?1",
                params![id.as_str()],
                |r| r.get(0),
            )
            .map_err(err)?;
        Ok(n > 0)
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, IndexError> {
        let limit = query.effective_limit();
        match (&query.text, &query.vector) {
            // Hybrid: fuse keyword and vector rankings via reciprocal rank fusion.
            (Some(_), Some(vector)) => {
                let kw = self.keyword_hits(query, limit * 4)?;
                let vec = self.vector_hits(query, vector, limit * 4)?;
                let kw_ids: Vec<BlockId> = kw.iter().map(|h| h.block.id.clone()).collect();
                let vec_ids: Vec<BlockId> = vec.iter().map(|h| h.block.id.clone()).collect();
                let mut records: std::collections::HashMap<BlockId, BlockRecord> =
                    std::collections::HashMap::new();
                for h in kw.into_iter().chain(vec) {
                    records.entry(h.block.id.clone()).or_insert(h.block);
                }
                let fused = mdkb_core::reciprocal_rank_fusion(&kw_ids, &vec_ids, 60.0);
                Ok(fused
                    .into_iter()
                    .filter_map(|(id, score)| {
                        records.remove(&id).map(|block| SearchHit { block, score })
                    })
                    .take(limit)
                    .collect())
            }
            (None, Some(vector)) => self.vector_hits(query, vector, limit),
            (Some(_), None) => self.keyword_hits(query, limit),
            (None, None) => self.filter_only_hits(query, limit),
        }
    }

    fn block(&self, id: &BlockId) -> Result<Option<BlockRecord>, IndexError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, page_path, kind, heading_level, lang, lineage, content, contextual_text, tags_text, 0.0 \
                 FROM blocks WHERE id = ?1",
            )
            .map_err(err)?;
        let mut rows = stmt
            .query_map(params![id.as_str()], row_to_record)
            .map_err(err)?;
        match rows.next() {
            Some(r) => Ok(Some(r.map_err(err)?)),
            None => Ok(None),
        }
    }

    fn links_from(&self, id: &BlockId) -> Result<Vec<LinkRow>, IndexError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT source_id, target_page, target_id, target_anchor, kind FROM links WHERE source_id = ?1",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![id.as_str()], row_to_link)
            .map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }

    fn backlinks(&self, id: &BlockId) -> Result<Vec<LinkRow>, IndexError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT source_id, target_page, target_id, target_anchor, kind FROM links WHERE target_id = ?1",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![id.as_str()], row_to_link)
            .map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }

    fn stats(&self) -> Result<IndexStats, IndexError> {
        let pages: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM pages", [], |r| r.get::<_, i64>(0))
            .map_err(err)? as usize;
        let blocks: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM blocks", [], |r| r.get::<_, i64>(0))
            .map_err(err)? as usize;
        let embedded: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM block_vectors", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize;
        Ok(IndexStats {
            pages,
            blocks,
            embedded,
        })
    }
}

fn row_to_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<BlockRecord> {
    let lineage: String = r.get(5)?;
    let tags_text: String = r.get(8)?;
    Ok(BlockRecord {
        id: BlockId::parse(&r.get::<_, String>(0)?).unwrap_or_else(|_| BlockId::generate()),
        page_path: r.get(1)?,
        kind: r.get(2)?,
        heading_level: r.get(3)?,
        lang: r.get(4)?,
        lineage: if lineage.is_empty() {
            Vec::new()
        } else {
            lineage.split(LINEAGE_SEP).map(|s| s.to_string()).collect()
        },
        tags: if tags_text.is_empty() {
            Vec::new()
        } else {
            tags_text.split(' ').map(|s| s.to_string()).collect()
        },
        content: r.get(6)?,
        contextual_text: r.get(7)?,
    })
}

fn row_to_link(r: &rusqlite::Row<'_>) -> rusqlite::Result<LinkRow> {
    let source: String = r.get(0)?;
    let target_id: Option<String> = r.get(2)?;
    let kind: String = r.get(4)?;
    Ok(LinkRow {
        source_id: BlockId::parse(&source).unwrap_or_else(|_| BlockId::generate()),
        target_page: r.get(1)?,
        target_id: target_id.and_then(|s| BlockId::parse(&s).ok()),
        target_anchor: r.get(3)?,
        kind: if kind == "transcludes" {
            LinkKind::Transcludes
        } else {
            LinkKind::References
        },
    })
}

/// Turn arbitrary user text into a safe FTS5 MATCH expression: alphanumeric tokens, each
/// quoted, joined by ` OR ` so a block matches when it contains *any* query term (ranked by
/// bm25). Implicit AND (joining by space) would require *every* term to be present, which
/// makes natural-language / paraphrased queries — where only some words overlap the block —
/// match nothing. Quoting each token avoids FTS5 syntax errors from punctuation.
fn to_fts_query(text: &str) -> String {
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect();
    tokens.join(" OR ")
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS pages (
    path TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS blocks (
    rowid           INTEGER PRIMARY KEY,
    id              TEXT UNIQUE NOT NULL,
    page_path       TEXT NOT NULL,
    kind            TEXT NOT NULL,
    heading_level   INTEGER,
    lang            TEXT,
    lineage         TEXT NOT NULL,
    content         TEXT NOT NULL,
    contextual_text TEXT NOT NULL,
    tags_text       TEXT NOT NULL,
    ord             INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_blocks_page ON blocks(page_path);

CREATE TABLE IF NOT EXISTS block_tags (
    block_id TEXT NOT NULL,
    tag      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tags_tag ON block_tags(tag);
CREATE INDEX IF NOT EXISTS idx_tags_block ON block_tags(block_id);

CREATE TABLE IF NOT EXISTS links (
    source_id     TEXT NOT NULL,
    target_page   TEXT,
    target_id     TEXT,
    target_anchor TEXT,
    kind          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_links_source ON links(source_id);
CREATE INDEX IF NOT EXISTS idx_links_target_id ON links(target_id);
CREATE INDEX IF NOT EXISTS idx_links_target_page ON links(target_page);

CREATE VIRTUAL TABLE IF NOT EXISTS block_fts USING fts5(
    content,
    lineage,
    tags,
    tokenize = 'porter unicode61'
);

CREATE TABLE IF NOT EXISTS block_vectors (
    block_id  TEXT PRIMARY KEY,
    model_id  TEXT NOT NULL,
    dim       INTEGER NOT NULL,
    embedding BLOB NOT NULL
);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use mdkb_core::{page_links, Vault};

    fn indexed_vault(pages: &[(&str, &str)]) -> (SqliteIndex, Vault) {
        let mut v = Vault::new();
        for (p, s) in pages {
            v.insert(*p, *s);
        }
        v.assign_ids();
        let mut idx = SqliteIndex::open_in_memory().unwrap();
        idx.rebuild(&v).unwrap();
        (idx, v)
    }

    #[test]
    fn keyword_search_finds_blocks() {
        let (idx, _v) = indexed_vault(&[(
            "ops.md",
            "# Nginx\n\nTo restart nginx run the bounce script.\n",
        )]);
        // Semantic-ish: searching different words that share the token "restart".
        let hits = idx.search(&SearchQuery::text("restart")).unwrap();
        assert!(hits
            .iter()
            .any(|h| h.block.content.contains("bounce script")));
    }

    #[test]
    fn to_fts_query_joins_terms_with_or() {
        // Locks OR semantics: a multi-term query must not require every term present.
        assert_eq!(
            to_fts_query("restart the server"),
            "\"restart\" OR \"the\" OR \"server\""
        );
        assert_eq!(to_fts_query("single"), "\"single\"");
        assert_eq!(to_fts_query("  -- ,. "), "");
    }

    #[test]
    fn keyword_search_matches_any_term_not_all() {
        // A paraphrased, multi-word query where only *some* terms appear in the block must
        // still match. Under the old implicit-AND join (`"a" "b" "c"`) this returned nothing
        // because "database" is absent; OR (`"a" OR "b" OR "c"`) ranks it via the shared terms.
        let (idx, _v) = indexed_vault(&[(
            "ops.md",
            "# Web server\n\nBounce the nginx service: systemctl restart nginx\n",
        )]);
        let hits = idx
            .search(&SearchQuery::text("restart the database server"))
            .unwrap();
        assert!(
            hits.iter().any(|h| h.block.content.contains("nginx")),
            "OR query should match a block that shares only some of the terms"
        );
    }

    #[test]
    fn search_filters_by_lang() {
        let (idx, _v) = indexed_vault(&[(
            "q.md",
            "# Queries\n\n```kusto\nStormEvents | take 10\n```\n\n```bash\nls -la\n```\n",
        )]);
        let q = SearchQuery {
            lang: Some("kusto".to_string()),
            ..Default::default()
        };
        let hits = idx.search(&q).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].block.content.contains("StormEvents"));
    }

    #[test]
    fn search_filters_by_tag() {
        let (idx, _v) = indexed_vault(&[("n.md", "alpha #keep\n\nbeta #drop\n")]);
        let q = SearchQuery {
            tags: vec!["keep".to_string()],
            ..Default::default()
        };
        let hits = idx.search(&q).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].block.content.contains("alpha"));
    }

    #[test]
    fn reindex_page_removes_stale_blocks() {
        let mut v = Vault::new();
        v.insert("a.md", "one\n\ntwo\n");
        v.assign_ids();
        let mut idx = SqliteIndex::open_in_memory().unwrap();
        idx.rebuild(&v).unwrap();
        assert_eq!(idx.stats().unwrap().blocks, 2);

        // Shrink the page to a single block and reindex.
        v.insert("a.md", "only one now\n");
        v.assign_ids();
        let page = v.page("a").unwrap().clone();
        let links = page_links(&v, &page);
        idx.reindex_page(&page, &links).unwrap();
        assert_eq!(idx.stats().unwrap().blocks, 1);
    }

    #[test]
    fn backlinks_track_transclusions() {
        let mut v = Vault::new();
        v.insert("src.md", "# Q\n\nthe query\n");
        v.assign_ids();
        let qid = v
            .page("src")
            .unwrap()
            .doc
            .blocks
            .iter()
            .find(|b| b.content.contains("the query"))
            .unwrap()
            .id
            .clone();
        v.insert("dst.md", format!("![[src#{qid}]]\n"));
        v.assign_ids();
        let mut idx = SqliteIndex::open_in_memory().unwrap();
        idx.rebuild(&v).unwrap();
        let back = idx.backlinks(&qid).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].kind, LinkKind::Transcludes);
    }

    #[test]
    fn remove_page_clears_everything() {
        let (mut idx, _v) = indexed_vault(&[("a.md", "hello\n")]);
        assert_eq!(idx.stats().unwrap().pages, 1);
        idx.remove_page("a.md").unwrap();
        let s = idx.stats().unwrap();
        assert_eq!(s.pages, 0);
        assert_eq!(s.blocks, 0);
    }

    #[test]
    fn vector_search_ranks_by_cosine() {
        use mdkb_core::{Embedder, HashEmbedder};
        let (mut idx, v) = indexed_vault(&[(
            "n.md",
            "# Server\n\nrestart the nginx web server\n\nfavourite pizza toppings\n",
        )]);
        let embedder = HashEmbedder::new(512);
        // Embed every block.
        for page in v.pages() {
            for b in &page.doc.blocks {
                let vec = embedder.embed_one(&b.contextual_text()).unwrap();
                idx.set_embedding(&b.id, &embedder.model_id(), &vec)
                    .unwrap();
            }
        }
        assert!(idx.stats().unwrap().embedded >= 2);

        let q = SearchQuery {
            vector: Some(embedder.embed_one("how do I restart nginx").unwrap()),
            ..Default::default()
        };
        let hits = idx.search(&q).unwrap();
        assert!(!hits.is_empty());
        // The nginx block should outrank the pizza block.
        assert!(hits[0].block.content.contains("nginx"));
    }

    #[test]
    fn vector_search_excludes_other_model_embeddings() {
        use mdkb_core::{Embedder, HashEmbedder};
        let (mut idx, v) = indexed_vault(&[("n.md", "restart the nginx server\n")]);
        let embedder = HashEmbedder::new(512);
        for page in v.pages() {
            for b in &page.doc.blocks {
                let vec = embedder.embed_one(&b.contextual_text()).unwrap();
                idx.set_embedding(&b.id, "model-A", &vec).unwrap();
            }
        }
        // Query tagged with a DIFFERENT model id must not match model-A vectors (no silent
        // cross-space comparison).
        let q = SearchQuery {
            vector: Some(embedder.embed_one("nginx").unwrap()),
            vector_model: Some("model-B".to_string()),
            ..Default::default()
        };
        assert!(idx.search(&q).unwrap().is_empty());
        // Same model id matches.
        let q_same = SearchQuery {
            vector: Some(embedder.embed_one("nginx").unwrap()),
            vector_model: Some("model-A".to_string()),
            ..Default::default()
        };
        assert!(!idx.search(&q_same).unwrap().is_empty());
    }

    #[test]
    fn hybrid_search_fuses_keyword_and_vector() {
        use mdkb_core::{Embedder, HashEmbedder};
        let (mut idx, v) = indexed_vault(&[(
            "n.md",
            "# Ops\n\nbounce the nginx service script\n\nunrelated grocery list\n",
        )]);
        let embedder = HashEmbedder::new(512);
        for page in v.pages() {
            for b in &page.doc.blocks {
                let vec = embedder.embed_one(&b.contextual_text()).unwrap();
                idx.set_embedding(&b.id, &embedder.model_id(), &vec)
                    .unwrap();
            }
        }
        let q = SearchQuery {
            text: Some("nginx".to_string()),
            vector: Some(embedder.embed_one("restart nginx service").unwrap()),
            ..Default::default()
        };
        let hits = idx.search(&q).unwrap();
        assert!(hits.iter().any(|h| h.block.content.contains("nginx")));
        assert!(hits[0].block.content.contains("nginx"));
    }

    #[test]
    fn reindex_drops_stale_embeddings() {
        use mdkb_core::{page_links, Embedder, HashEmbedder};
        let mut v = Vault::new();
        v.insert("a.md", "alpha block\n\nbeta block\n");
        v.assign_ids();
        let mut idx = SqliteIndex::open_in_memory().unwrap();
        idx.rebuild(&v).unwrap();
        let embedder = HashEmbedder::new(64);
        for b in &v.page("a").unwrap().doc.blocks {
            idx.set_embedding(
                &b.id,
                &embedder.model_id(),
                &embedder.embed_one(&b.content).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(idx.stats().unwrap().embedded, 2);
        // Shrink page; reindex clears the page's embeddings (re-embedding is the engine's
        // job), so the stale "beta" embedding cannot linger.
        v.insert("a.md", "alpha block\n");
        v.assign_ids();
        let page = v.page("a").unwrap().clone();
        idx.reindex_page(&page, &page_links(&v, &page)).unwrap();
        assert_eq!(idx.stats().unwrap().embedded, 0);
        // Re-embedding the surviving block yields exactly one embedding.
        for b in &page.doc.blocks {
            idx.set_embedding(
                &b.id,
                &embedder.model_id(),
                &embedder.embed_one(&b.content).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(idx.stats().unwrap().embedded, 1);
    }

    #[test]
    fn engine_semantic_search_end_to_end() {
        use mdkb_core::{HashEmbedder, SyncEngine};
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("ops.md"),
            "# Web server\n\nbounce the nginx service to apply config\n\npicking ripe avocados\n",
        )
        .unwrap();

        let idx = SqliteIndex::open_in_memory().unwrap();
        let mut engine =
            SyncEngine::new(root.path(), idx).with_embedder(Box::new(HashEmbedder::new(512)));
        engine.reconcile().unwrap();
        assert!(engine.index().stats().unwrap().embedded >= 2);

        // A semantically related query with no shared keywords beyond "nginx".
        let hits = engine
            .search(SearchQuery {
                text: Some("restart nginx".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].block.content.contains("nginx"));
    }
}
