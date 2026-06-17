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
            "DELETE FROM blocks WHERE page_path = ?1",
            params![page_path],
        )
        .map_err(err)?;
        tx.execute("DELETE FROM pages WHERE path = ?1", params![page_path])
            .map_err(err)?;
        Ok(())
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
                "DELETE FROM block_fts; DELETE FROM block_tags; DELETE FROM links; DELETE FROM blocks; DELETE FROM pages;",
            )
            .map_err(err)
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, IndexError> {
        let limit = query.effective_limit() as i64;
        let mut sql = String::new();
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(text) = &query.text {
            let match_expr = to_fts_query(text);
            if match_expr.is_empty() {
                return Ok(Vec::new());
            }
            sql.push_str(
                "SELECT b.id, b.page_path, b.kind, b.heading_level, b.lang, b.lineage, b.content, b.contextual_text, b.tags_text, \
                 bm25(block_fts) AS rank \
                 FROM block_fts JOIN blocks b ON b.rowid = block_fts.rowid \
                 WHERE block_fts MATCH ?1",
            );
            args.push(Box::new(match_expr));
        } else {
            sql.push_str(
                "SELECT b.id, b.page_path, b.kind, b.heading_level, b.lang, b.lineage, b.content, b.contextual_text, b.tags_text, \
                 0.0 AS rank FROM blocks b WHERE 1=1",
            );
        }

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

        if query.text.is_some() {
            sql.push_str(" ORDER BY rank LIMIT ?");
        } else {
            sql.push_str(" ORDER BY b.page_path, b.ord LIMIT ?");
        }
        args.push(Box::new(limit));
        let limit_idx = args.len();
        sql.push_str(&limit_idx.to_string());

        let mut stmt = self.conn.prepare(&sql).map_err(err)?;
        let rows = stmt
            .query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |r| {
                let rank: f64 = r.get(9)?;
                // bm25 returns lower-is-better; flip so higher score = more relevant.
                let score = if rank == 0.0 { 0.0 } else { -rank };
                Ok(SearchHit {
                    block: row_to_record(r)?,
                    score,
                })
            })
            .map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
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
/// quoted, joined by space (implicit AND). Avoids FTS5 syntax errors from punctuation.
fn to_fts_query(text: &str) -> String {
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect();
    tokens.join(" ")
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
}
