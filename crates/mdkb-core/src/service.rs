//! The mdkb service API: the single, shared surface every client uses.
//!
//! The daemon's transport handlers, the MCP server, and the CLI all call into [`Service`]
//! — they never re-implement block, search, or write behavior themselves. This is the
//! concrete realisation of the "one shared core, no divergence" rule in `AGENTS.md`: a bug
//! fixed here is fixed for every client at once.
//!
//! Every method takes a [`RequestContext`] carrying the [`Caller`], and write methods are
//! gated through [`RequestContext::authorize`]. Today `Caller::Local` is allowed
//! everything; the seam exists so network deployments can add real authz without touching
//! call sites (see plan Decision #9).

use crate::id::BlockId;
use crate::index::{BlockRecord, Index, IndexError, IndexStats, LinkRow, SearchHit, SearchQuery};
use crate::render::render_page;
use crate::sync::SyncEngine;

/// Who is making a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// A local, fully-trusted caller (same machine, Unix socket / in-process).
    Local,
    /// A network caller that presented a valid shared token (opaque principal).
    Authenticated(String),
    /// A remote caller that has not authenticated (network deployments).
    Remote(String),
}

/// A capability a request needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Read-only access.
    Read,
    /// Mutating access.
    Write,
}

/// Per-request context. Carries identity and is the hook for future authorization.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// The caller.
    pub caller: Caller,
}

impl RequestContext {
    /// A local, trusted context.
    pub fn local() -> Self {
        RequestContext {
            caller: Caller::Local,
        }
    }

    /// A remote context with an opaque principal id.
    pub fn remote(id: impl Into<String>) -> Self {
        RequestContext {
            caller: Caller::Remote(id.into()),
        }
    }

    /// A network context that has presented a valid shared token.
    pub fn authenticated(id: impl Into<String>) -> Self {
        RequestContext {
            caller: Caller::Authenticated(id.into()),
        }
    }

    /// Authorize `cap` for this caller. Local and token-authenticated callers are permitted
    /// everything today; un-authenticated remote callers fail closed. This is the single
    /// choke point where finer-grained (e.g. read-only token) authz will be enforced later.
    pub fn authorize(&self, _cap: Capability) -> Result<(), IndexError> {
        match &self.caller {
            Caller::Local | Caller::Authenticated(_) => Ok(()),
            Caller::Remote(id) => Err(IndexError::new(format!(
                "unauthorized: remote caller {id} (authenticate with a token first)"
            ))),
        }
    }
}

/// The mdkb service: wraps a [`SyncEngine`] and exposes the deterministic API.
pub struct Service<I: Index> {
    engine: SyncEngine<I>,
}

impl<I: Index> Service<I> {
    /// Wrap a sync engine.
    pub fn new(engine: SyncEngine<I>) -> Self {
        Service { engine }
    }

    /// Borrow the underlying engine (read-only).
    pub fn engine(&self) -> &SyncEngine<I> {
        &self.engine
    }

    /// Mutably borrow the underlying engine (e.g. for the watcher to reconcile).
    pub fn engine_mut(&mut self) -> &mut SyncEngine<I> {
        &mut self.engine
    }

    // ----- reads -----

    /// Search the index (keyword + semantic, fused).
    pub fn search(
        &self,
        ctx: &RequestContext,
        query: SearchQuery,
    ) -> Result<Vec<SearchHit>, IndexError> {
        ctx.authorize(Capability::Read)?;
        self.engine.search(query)
    }

    /// Fetch a single block record by id.
    pub fn get_block(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
    ) -> Result<Option<BlockRecord>, IndexError> {
        ctx.authorize(Capability::Read)?;
        self.engine.index().block(id)
    }

    /// Get the raw Markdown source of a page.
    pub fn get_page_source(
        &self,
        ctx: &RequestContext,
        page: &str,
    ) -> Result<Option<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(self.engine.vault().page(page).map(|p| p.doc.source.clone()))
    }

    /// Render a page with all transclusions resolved.
    pub fn render_page(
        &self,
        ctx: &RequestContext,
        page: &str,
    ) -> Result<Option<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(render_page(self.engine.vault(), page))
    }

    /// Outgoing links from a block.
    pub fn links_from(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
    ) -> Result<Vec<LinkRow>, IndexError> {
        ctx.authorize(Capability::Read)?;
        self.engine.index().links_from(id)
    }

    /// Incoming references / transclusions of a block.
    pub fn backlinks(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
    ) -> Result<Vec<LinkRow>, IndexError> {
        ctx.authorize(Capability::Read)?;
        self.engine.index().backlinks(id)
    }

    /// List page paths.
    pub fn list_pages(&self, ctx: &RequestContext) -> Result<Vec<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        let mut pages = self.engine.page_paths();
        pages.sort();
        Ok(pages)
    }

    /// Index statistics.
    pub fn stats(&self, ctx: &RequestContext) -> Result<IndexStats, IndexError> {
        ctx.authorize(Capability::Read)?;
        self.engine.index().stats()
    }

    // ----- writes -----

    /// Upsert a block: if `id` is given, replace that block's text; otherwise append a new
    /// block to `page` (creating the page if needed). Returns the affected block id.
    pub fn upsert_block(
        &mut self,
        ctx: &RequestContext,
        id: Option<BlockId>,
        text: &str,
        page: Option<&str>,
    ) -> Result<BlockId, IndexError> {
        ctx.authorize(Capability::Write)?;
        match id {
            Some(id) => {
                let ok = self.engine.update_block(&id, text)?;
                if ok {
                    Ok(id)
                } else {
                    Err(IndexError::new(format!("unknown block id: {id}")))
                }
            }
            None => {
                let page = page.ok_or_else(|| {
                    IndexError::new("upsert_block requires a page when no id is given")
                })?;
                self.engine.append_block(page, text)
            }
        }
    }

    /// Save a whole page (create or overwrite) from raw Markdown.
    pub fn save_page(
        &mut self,
        ctx: &RequestContext,
        page: &str,
        source: &str,
    ) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.save_page(page, source)
    }

    /// Delete a page (file + index).
    pub fn delete_page(&mut self, ctx: &RequestContext, page: &str) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.delete_page(page)
    }

    /// Create a link or transclusion from `source_id` to a target, by appending the wiki
    /// directive to the source block's text. `embed = true` writes `![[...]]`
    /// (transclusion); `false` writes `[[...]]` (reference).
    pub fn link_blocks(
        &mut self,
        ctx: &RequestContext,
        source_id: &BlockId,
        target_page: Option<&str>,
        target_id: Option<&BlockId>,
        target_anchor: Option<&str>,
        embed: bool,
    ) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        let current = self
            .engine
            .index()
            .block(source_id)?
            .ok_or_else(|| IndexError::new(format!("unknown source block: {source_id}")))?;
        let directive = build_directive(target_page, target_id, target_anchor, embed)?;
        let new_text = format!("{}\n\n{}", current.content.trim_end(), directive);
        self.engine.update_block(source_id, &new_text)?;
        Ok(())
    }

    /// Reconcile the vault directory with the index (used on startup and by the watcher).
    pub fn reconcile(
        &mut self,
        ctx: &RequestContext,
    ) -> Result<crate::sync::SyncReport, IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.reconcile()
    }

    /// Rebuild the entire index from the vault files (clear + re-ingest everything).
    pub fn rebuild(&mut self, ctx: &RequestContext) -> Result<crate::sync::SyncReport, IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.rebuild()
    }

    /// Cloud-sync conflict files detected at the last reconcile (surfaced, not indexed).
    pub fn conflicts(&self, ctx: &RequestContext) -> Result<Vec<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(self.engine.conflicts().to_vec())
    }
}

fn build_directive(
    page: Option<&str>,
    id: Option<&BlockId>,
    anchor: Option<&str>,
    embed: bool,
) -> Result<String, IndexError> {
    let mut inner = String::new();
    if let Some(p) = page {
        inner.push_str(p.strip_suffix(".md").unwrap_or(p));
    }
    let anchor_str = id
        .map(|i| i.to_string())
        .or_else(|| anchor.map(|a| a.to_string()));
    if let Some(a) = anchor_str {
        inner.push('#');
        inner.push_str(&a);
    }
    if inner.is_empty() {
        return Err(IndexError::new("link target is empty"));
    }
    Ok(if embed {
        format!("![[{inner}]]")
    } else {
        format!("[[{inner}]]")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::testing::MemIndex;
    use crate::SyncEngine;

    fn service(root: &std::path::Path) -> Service<MemIndex> {
        let engine = SyncEngine::new(root, MemIndex::default());
        Service::new(engine)
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();

        // Create a page with a block.
        let id = svc
            .upsert_block(&ctx, None, "the original query", Some("queries.md"))
            .unwrap();
        assert!(dir.path().join("queries.md").exists());

        // Read it back.
        let block = svc.get_block(&ctx, &id).unwrap().unwrap();
        assert_eq!(block.content, "the original query");

        // Update in place.
        svc.upsert_block(&ctx, Some(id.clone()), "the updated query", None)
            .unwrap();
        assert_eq!(
            svc.get_block(&ctx, &id).unwrap().unwrap().content,
            "the updated query"
        );
    }

    #[test]
    fn link_blocks_creates_transclusion_reflected_in_render() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();

        let qid = svc
            .upsert_block(&ctx, None, "StormEvents | take 10", Some("queries.md"))
            .unwrap();
        let host = svc
            .upsert_block(&ctx, None, "Project notes:", Some("project.md"))
            .unwrap();

        // Link project block -> query block as an embed.
        svc.link_blocks(&ctx, &host, Some("queries.md"), Some(&qid), None, true)
            .unwrap();

        // Rendering the project page inlines the transcluded query.
        let rendered = svc.render_page(&ctx, "project.md").unwrap().unwrap();
        assert!(rendered.contains("StormEvents | take 10"));
    }

    #[test]
    fn remote_caller_is_denied_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path());
        let ctx = RequestContext::remote("agent-7");
        assert!(
            svc.list_pages(&ctx).is_err(),
            "remote callers must fail closed"
        );
    }

    #[test]
    fn delete_page_removes_file_and_index() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        svc.save_page(&ctx, "a.md", "hello\n").unwrap();
        assert_eq!(svc.stats(&ctx).unwrap().pages, 1);
        svc.delete_page(&ctx, "a.md").unwrap();
        assert_eq!(svc.stats(&ctx).unwrap().pages, 0);
        assert!(!dir.path().join("a.md").exists());
    }
}
