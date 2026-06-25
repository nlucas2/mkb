//! SQLite implementation of [`mdkb_core::Index`] for the file-per-block model.
//!
//! Bundled SQLite (no system dependency) with an FTS5 virtual table for keyword search and a
//! per-block vector store for semantic search. The unit is the **block** (one file). The index
//! is a **rebuildable cache** of the `blocks/` directory: it can be thrown away and
//! reconstructed from the files at any time, so it is never the source of truth.

use std::path::Path;

use mdkb_core::{
    BlockId, BlockRecord, Index, IndexError, IndexStats, LinkKind, LinkRow, SearchHit, SearchQuery,
    TagCount,
};
use rusqlite::{params, params_from_iter, Connection};

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

    /// Borrow the underlying connection (for tests).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    fn delete_block_rows(tx: &Connection, id: &str) -> Result<(), IndexError> {
        // Remove the FTS row (by rowid) before deleting the backing block row.
        let rowid: Option<i64> = tx
            .query_row("SELECT rowid FROM blocks WHERE id = ?1", params![id], |r| {
                r.get(0)
            })
            .ok();
        if let Some(rid) = rowid {
            tx.execute("DELETE FROM block_fts WHERE rowid = ?1", params![rid])
                .map_err(err)?;
        }
        tx.execute("DELETE FROM blocks WHERE id = ?1", params![id])
            .map_err(err)?;
        tx.execute("DELETE FROM block_tags WHERE block_id = ?1", params![id])
            .map_err(err)?;
        tx.execute("DELETE FROM links WHERE source_id = ?1", params![id])
            .map_err(err)?;
        Ok(())
    }

    /// Append filter clauses (tags AND, lang) shared by the search paths.
    fn push_filters(
        sql: &mut String,
        args: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
        query: &SearchQuery,
    ) {
        if let Some(lang) = &query.lang {
            args.push(Box::new(lang.to_lowercase()));
            sql.push_str(&format!(
                " AND b.id IN (SELECT block_id FROM block_langs WHERE lang = ?{})",
                args.len()
            ));
        }
        for tag in &query.tags {
            args.push(Box::new(tag.to_lowercase()));
            sql.push_str(&format!(
                " AND b.id IN (SELECT block_id FROM block_tags WHERE tag = ?{})",
                args.len()
            ));
        }
    }

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
            "SELECT b.id, b.title, b.langs_text, b.content, b.contextual_text, b.tags_text, b.child_count, \
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
                let rank: f64 = r.get(7)?;
                Ok(SearchHit {
                    block: row_to_record(r)?,
                    score: -rank,
                })
            })
            .map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }

    fn vector_hits(
        &self,
        query: &SearchQuery,
        vector: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        let mut sql = String::from(
            "SELECT b.id, b.title, b.langs_text, b.content, b.contextual_text, b.tags_text, b.child_count, \
             v.embedding \
             FROM blocks b JOIN block_vectors v ON v.block_id = b.id WHERE 1=1",
        );
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(model) = &query.vector_model {
            args.push(Box::new(model.clone()));
            sql.push_str(&format!(" AND v.model_id = ?{}", args.len()));
        }
        Self::push_filters(&mut sql, &mut args, query);

        let mut stmt = self.conn.prepare(&sql).map_err(err)?;
        let rows = stmt
            .query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |r| {
                let blob: Vec<u8> = r.get(7)?;
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

    fn filter_only_hits(
        &self,
        query: &SearchQuery,
        limit: usize,
    ) -> Result<Vec<SearchHit>, IndexError> {
        let mut sql = String::from(
            "SELECT b.id, b.title, b.langs_text, b.content, b.contextual_text, b.tags_text, b.child_count, \
             0.0 FROM blocks b WHERE 1=1",
        );
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        Self::push_filters(&mut sql, &mut args, query);
        args.push(Box::new(limit as i64));
        sql.push_str(&format!(" ORDER BY b.id LIMIT ?{}", args.len()));

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
    fn reindex_block(&mut self, record: &BlockRecord, links: &[LinkRow]) -> Result<(), IndexError> {
        let tx = self.conn.transaction().map_err(err)?;
        Self::delete_block_rows(&tx, record.id.as_str())?;

        let tags_text = record.tags.join(" ");
        let langs_text = record.langs.join(" ");
        tx.execute(
            "INSERT INTO blocks(id, title, content, contextual_text, tags_text, langs_text, child_count) \
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                record.id.as_str(),
                record.title,
                record.content,
                record.contextual_text,
                tags_text,
                langs_text,
                record.child_count as i64,
            ],
        )
        .map_err(err)?;
        let rowid = tx.last_insert_rowid();

        // FTS row: content + title + tags (so all are searchable).
        tx.execute(
            "INSERT INTO block_fts(rowid, content, title, tags) VALUES (?1,?2,?3,?4)",
            params![rowid, record.contextual_text, record.title, tags_text],
        )
        .map_err(err)?;

        for tag in &record.tags {
            tx.execute(
                "INSERT INTO block_tags(block_id, tag) VALUES (?1,?2)",
                params![record.id.as_str(), tag.to_lowercase()],
            )
            .map_err(err)?;
        }
        for lang in &record.langs {
            tx.execute(
                "INSERT INTO block_langs(block_id, lang) VALUES (?1,?2)",
                params![record.id.as_str(), lang.to_lowercase()],
            )
            .map_err(err)?;
        }
        for link in links {
            tx.execute(
                "INSERT INTO links(source_id, target_id, target, kind) VALUES (?1,?2,?3,?4)",
                params![
                    link.source_id.as_str(),
                    link.target_id.as_ref().map(|t| t.as_str()),
                    link.target,
                    link.kind.as_str(),
                ],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(())
    }

    fn remove_block(&mut self, id: &BlockId) -> Result<(), IndexError> {
        let tx = self.conn.transaction().map_err(err)?;
        Self::delete_block_rows(&tx, id.as_str())?;
        tx.execute(
            "DELETE FROM block_langs WHERE block_id = ?1",
            params![id.as_str()],
        )
        .map_err(err)?;
        tx.execute(
            "DELETE FROM block_vectors WHERE block_id = ?1",
            params![id.as_str()],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(())
    }

    fn clear(&mut self) -> Result<(), IndexError> {
        self.conn
            .execute_batch(
                "DELETE FROM block_fts; DELETE FROM blocks; DELETE FROM block_tags; \
                 DELETE FROM block_langs; DELETE FROM links; DELETE FROM block_vectors;",
            )
            .map_err(err)
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, IndexError> {
        let limit = query.effective_limit();
        match (&query.text, &query.vector) {
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
                "SELECT id, title, langs_text, content, contextual_text, tags_text, child_count, 0.0 \
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
            .prepare("SELECT source_id, target_id, target, kind FROM links WHERE source_id = ?1")
            .map_err(err)?;
        let rows = stmt
            .query_map(params![id.as_str()], row_to_link)
            .map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }

    fn backlinks(&self, id: &BlockId) -> Result<Vec<LinkRow>, IndexError> {
        let mut stmt = self
            .conn
            .prepare("SELECT source_id, target_id, target, kind FROM links WHERE target_id = ?1")
            .map_err(err)?;
        let rows = stmt
            .query_map(params![id.as_str()], row_to_link)
            .map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }

    fn stats(&self) -> Result<IndexStats, IndexError> {
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
            blocks,
            roots: 0, // filled in by the Service from the vault
            embedded,
        })
    }

    fn tag_counts(&self) -> Result<Vec<TagCount>, IndexError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT tag, COUNT(*) AS n FROM block_tags GROUP BY tag \
                 ORDER BY n DESC, tag ASC",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(TagCount {
                    tag: r.get::<_, String>(0)?,
                    count: r.get::<_, i64>(1)? as usize,
                })
            })
            .map_err(err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(err)?);
        }
        Ok(out)
    }

    fn set_embedding(
        &mut self,
        id: &BlockId,
        model_id: &str,
        vector: &[f32],
    ) -> Result<(), IndexError> {
        let blob = mdkb_core::vector_to_bytes(vector);
        self.conn
            .execute(
                "INSERT INTO block_vectors(block_id, model_id, dim, embedding) VALUES (?1,?2,?3,?4) \
                 ON CONFLICT(block_id) DO UPDATE SET model_id=?2, dim=?3, embedding=?4",
                params![id.as_str(), model_id, vector.len() as i64, blob],
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
}

fn row_to_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<BlockRecord> {
    let title: Option<String> = r.get(1)?;
    let langs_text: String = r.get(2)?;
    let tags_text: String = r.get(5)?;
    Ok(BlockRecord {
        id: BlockId::parse(&r.get::<_, String>(0)?).unwrap_or_else(|_| BlockId::generate()),
        title,
        langs: split_ws(&langs_text),
        content: r.get(3)?,
        contextual_text: r.get(4)?,
        tags: split_ws(&tags_text),
        child_count: r.get::<_, i64>(6)? as usize,
        // `locked`, `props`, `created`, and `updated` are not persisted in the index; the service
        // overlays them from the vault / the block id (the source of truth).
        locked: false,
        props: Vec::new(),
        created: None,
        updated: None,
    })
}

fn split_ws(s: &str) -> Vec<String> {
    if s.trim().is_empty() {
        Vec::new()
    } else {
        s.split_whitespace().map(|x| x.to_string()).collect()
    }
}

fn row_to_link(r: &rusqlite::Row<'_>) -> rusqlite::Result<LinkRow> {
    let source: String = r.get(0)?;
    let target_id: Option<String> = r.get(1)?;
    let kind: String = r.get(3)?;
    Ok(LinkRow {
        source_id: BlockId::parse(&source).unwrap_or_else(|_| BlockId::generate()),
        target_id: target_id.and_then(|s| BlockId::parse(&s).ok()),
        target: r.get(2)?,
        kind: if kind == "transcludes" {
            LinkKind::Transcludes
        } else {
            LinkKind::References
        },
    })
}

/// Turn arbitrary user text into a safe FTS5 MATCH expression.
///
/// - **Bare words** are each alphanumeric-tokenised, quoted, and joined by ` OR `, so a block
///   matches when it contains *any* term (ranked by bm25). Implicit AND (joining by space) makes
///   paraphrased queries match nothing; quoting each token avoids FTS5 syntax errors from
///   punctuation. This is the long-standing default (see the OR-vs-AND verdict in the design notes).
/// - A **double-quoted span** becomes a single FTS5 **phrase** — its tokens must appear in sequence
///   — so a sentence copied from the rendered view can be matched verbatim (`"exact phrase"`).
///   Markdown markers in the stored text (`**bold**`) don't interfere: both sides tokenise the same.
///
/// A phrase and any surrounding bare words are OR-joined together, keeping bare-word behaviour
/// unchanged. An unterminated quote treats the trailing text as a phrase.
fn to_fts_query(text: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    // Splitting on '"' alternates: even segments are outside quotes, odd segments are inside.
    for (i, segment) in text.split('"').enumerate() {
        let tokens: Vec<&str> = segment
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.is_empty() {
            continue;
        }
        if i % 2 == 1 {
            // Inside quotes → one ordered phrase term.
            parts.push(format!("\"{}\"", tokens.join(" ")));
        } else {
            // Outside quotes → each token is its own OR'd term.
            parts.extend(tokens.into_iter().map(|t| format!("\"{t}\"")));
        }
    }
    parts.join(" OR ")
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS blocks (
    rowid           INTEGER PRIMARY KEY,
    id              TEXT UNIQUE NOT NULL,
    title           TEXT,
    content         TEXT NOT NULL,
    contextual_text TEXT NOT NULL,
    tags_text       TEXT NOT NULL,
    langs_text      TEXT NOT NULL,
    child_count     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS block_tags (
    block_id TEXT NOT NULL,
    tag      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tags_tag ON block_tags(tag);
CREATE INDEX IF NOT EXISTS idx_tags_block ON block_tags(block_id);

CREATE TABLE IF NOT EXISTS block_langs (
    block_id TEXT NOT NULL,
    lang     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_langs_lang ON block_langs(lang);
CREATE INDEX IF NOT EXISTS idx_langs_block ON block_langs(block_id);

CREATE TABLE IF NOT EXISTS links (
    source_id TEXT NOT NULL,
    target_id TEXT,
    target    TEXT NOT NULL,
    kind      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_links_source ON links(source_id);
CREATE INDEX IF NOT EXISTS idx_links_target_id ON links(target_id);

CREATE VIRTUAL TABLE IF NOT EXISTS block_fts USING fts5(
    content,
    title,
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
    use mdkb_core::{block_links, BlockRecord, Vault};

    fn indexed(blocks: &[&str]) -> (SqliteIndex, Vault, Vec<BlockId>) {
        let mut v = Vault::new();
        let mut ids = Vec::new();
        for src in blocks {
            let id = BlockId::generate();
            v.insert_source(id.clone(), src);
            ids.push(id);
        }
        let mut idx = SqliteIndex::open_in_memory().unwrap();
        idx.rebuild(&v).unwrap();
        (idx, v, ids)
    }

    #[test]
    fn keyword_search_finds_block() {
        let (idx, _v, _ids) =
            indexed(&["---\ntitle: Nginx\n---\nrestart the web server with systemctl\n"]);
        let hits = idx.search(&SearchQuery::text("restart server")).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].block.content.contains("systemctl"));
    }

    #[test]
    fn or_join_matches_paraphrase() {
        // Only one of the query terms is present, but OR-join + bm25 still finds it.
        let (idx, _v, _ids) = indexed(&["reboot the machine cleanly\n"]);
        let hits = idx
            .search(&SearchQuery::text("restart the machine"))
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn quoted_phrase_matches_sequence_only() {
        // Two blocks contain all the same words, but only one has them in sequence.
        let (idx, _v, ids) = indexed(&[
            "the cat sat on the mat quietly\n",        // ordered
            "the mat was where the cat finally sat\n", // same words, scattered
        ]);
        // A bare (OR) query matches both blocks.
        let loose = idx
            .search(&SearchQuery::text("cat sat on the mat"))
            .unwrap();
        assert_eq!(loose.len(), 2);
        // The same query as a phrase matches only the in-order block.
        let phrase = idx
            .search(&SearchQuery::text("\"cat sat on the mat\""))
            .unwrap();
        assert_eq!(phrase.len(), 1);
        assert_eq!(phrase[0].block.id, ids[0]);
    }

    #[test]
    fn quoted_phrase_ignores_markdown_in_stored_text() {
        // Stored text has bold markers; a phrase copied from the rendered view (no markers) matches.
        let (idx, _v, _ids) = indexed(&["A fact lives in **exactly one block**.\n"]);
        let hits = idx
            .search(&SearchQuery::text("\"exactly one block\""))
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn filters_by_tag_and_lang() {
        let (idx, _v, _ids) = indexed(&[
            "---\ntitle: A\ntags: [ops]\n---\n```kusto\nStormEvents\n```\n",
            "---\ntitle: B\ntags: [notes]\n---\njust prose\n",
        ]);
        let by_tag = idx
            .search(&SearchQuery {
                tags: vec!["ops".into()],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(by_tag.len(), 1);
        let by_lang = idx
            .search(&SearchQuery {
                lang: Some("kusto".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(by_lang.len(), 1);
    }

    #[test]
    fn prop_values_are_full_text_searchable() {
        // A block's arbitrary property value is folded into contextual_text, so FTS finds it
        // even though it appears in frontmatter, not the body.
        let (idx, _v, ids) =
            indexed(&["---\ntitle: Atom\nsource: quokkaprovenance\n---\nan unremarkable body\n"]);
        let hits = idx.search(&SearchQuery::text("quokkaprovenance")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].block.id, ids[0]);
    }

    #[test]
    fn backlinks_track_transclusions() {
        let mut v = Vault::new();
        let child = BlockId::generate();
        let parent = BlockId::generate();
        v.insert_source(child.clone(), "---\ntitle: Child\n---\nx\n");
        v.insert_source(parent.clone(), &format!("![[{child}]]\n"));
        let mut idx = SqliteIndex::open_in_memory().unwrap();
        idx.rebuild(&v).unwrap();
        let back = idx.backlinks(&child).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].source_id, parent);
        assert_eq!(back[0].kind, LinkKind::Transcludes);
    }

    #[test]
    fn remove_block_clears_everything() {
        let (mut idx, _v, ids) = indexed(&["alpha #tag\n"]);
        assert_eq!(idx.stats().unwrap().blocks, 1);
        idx.remove_block(&ids[0]).unwrap();
        assert_eq!(idx.stats().unwrap().blocks, 0);
        assert!(idx.search(&SearchQuery::text("alpha")).unwrap().is_empty());
    }

    #[test]
    fn reindex_replaces_stale_content() {
        let id = BlockId::generate();
        let mut v = Vault::new();
        v.insert_source(id.clone(), "original text\n");
        let mut idx = SqliteIndex::open_in_memory().unwrap();
        idx.rebuild(&v).unwrap();
        // Update the block and reindex just it.
        v.insert_source(id.clone(), "replacement words\n");
        let rec = BlockRecord::from_block(v.block(&id).unwrap(), 0);
        idx.reindex_block(&rec, &block_links(&v, v.block(&id).unwrap()))
            .unwrap();
        assert!(idx.search(&SearchQuery::text("replacement")).unwrap().len() == 1);
        assert!(idx
            .search(&SearchQuery::text("original"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn to_fts_query_or_joins_and_sanitizes() {
        assert_eq!(
            to_fts_query("restart the server"),
            "\"restart\" OR \"the\" OR \"server\""
        );
        assert_eq!(to_fts_query("single"), "\"single\"");
        assert_eq!(to_fts_query("  -- ,. "), "");
    }

    #[test]
    fn to_fts_query_supports_quoted_phrases() {
        // A quoted span becomes one ordered FTS5 phrase, not OR'd tokens.
        assert_eq!(
            to_fts_query("\"a fact lives in exactly one block\""),
            "\"a fact lives in exactly one block\""
        );
        // A phrase plus surrounding bare words: phrase stays intact, bare words OR-joined.
        assert_eq!(
            to_fts_query("\"exact phrase\" loose word"),
            "\"exact phrase\" OR \"loose\" OR \"word\""
        );
        // An unterminated quote treats the trailing text as a phrase.
        assert_eq!(to_fts_query("foo \"bar baz"), "\"foo\" OR \"bar baz\"");
        // Markdown punctuation inside a phrase is tokenised the same way the stored text is.
        assert_eq!(
            to_fts_query("\"exactly **one** block\""),
            "\"exactly one block\""
        );
    }
}
