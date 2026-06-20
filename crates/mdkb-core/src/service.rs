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
use crate::index::{
    block_links, link_graph, transclusion_reaches, BlockRecord, GraphData, Index, IndexError,
    IndexStats, LinkOutcome, LinkRow, SearchHit, SearchQuery, TagCount,
};
use crate::render::{render_block, render_flat, rendered_block, RenderedBlock};
use crate::sync::{SyncEngine, SyncReport};

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

/// The mdkb service: wraps a [`SyncEngine`] and exposes the deterministic block API.
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

    /// The raw Markdown body of a block (for editing).
    pub fn get_block_source(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
    ) -> Result<Option<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(self.engine.vault().block(id).map(|b| b.body.clone()))
    }

    /// The block's optional title.
    pub fn get_block_title(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
    ) -> Result<Option<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(self.engine.vault().block(id).and_then(|b| b.title.clone()))
    }

    /// Render a block with all transclusions resolved (Markdown out).
    pub fn render_block(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
    ) -> Result<Option<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(render_block(self.engine.vault(), id))
    }

    /// Render a block as a [`RenderedBlock`] (raw + resolved Markdown).
    pub fn rendered_block(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
    ) -> Result<Option<RenderedBlock>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(rendered_block(self.engine.vault(), id))
    }

    /// Render a block to **flat, self-contained Markdown** (embeds dissolved inline, references
    /// as plain titles) — the published form used by export. Returns `None` if unknown.
    pub fn render_flat(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
    ) -> Result<Option<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(render_flat(self.engine.vault(), id))
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

    /// List all block ids (sorted).
    pub fn list_blocks(&self, ctx: &RequestContext) -> Result<Vec<BlockId>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(self.engine.vault().ids())
    }

    /// List root block ids (top-level entries; nothing transcludes them).
    pub fn list_roots(&self, ctx: &RequestContext) -> Result<Vec<BlockId>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(self.engine.vault().roots())
    }

    /// Index statistics.
    pub fn stats(&self, ctx: &RequestContext) -> Result<IndexStats, IndexError> {
        ctx.authorize(Capability::Read)?;
        let mut s = self.engine.index().stats()?;
        s.roots = self.engine.vault().roots().len();
        Ok(s)
    }

    /// The block-level knowledge graph.
    pub fn graph(&self, ctx: &RequestContext) -> Result<GraphData, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(link_graph(self.engine.vault()))
    }

    /// All tags in the vault with the number of blocks carrying each (for tag discovery).
    pub fn list_tags(&self, ctx: &RequestContext) -> Result<Vec<TagCount>, IndexError> {
        ctx.authorize(Capability::Read)?;
        self.engine.index().tag_counts()
    }

    /// Plan the docs-as-data export against the live vault: returns, for each output, the path and
    /// the exact content it should contain. With `Some(manifest_text)`, exports the mapped blocks;
    /// with `None`, exports **every root block** to `<slug>.md` (the no-manifest whole-KB dump).
    /// `raw` sets the banner policy for the whole-KB case. Rendering and banner logic live in
    /// [`crate::export`] so every client produces identical files.
    pub fn plan_exports(
        &self,
        ctx: &RequestContext,
        request: &crate::export::ExportRequest,
    ) -> Result<Vec<crate::export::PlannedDoc>, IndexError> {
        ctx.authorize(Capability::Read)?;
        let manifest = crate::export::manifest_for_request(self.engine.vault(), request);
        crate::export::plan_exports(self.engine.vault(), &manifest).map_err(IndexError::new)
    }

    /// All link rows in the vault that are dangling (unresolved target) — for the health view.
    pub fn dangling_links(&self, ctx: &RequestContext) -> Result<Vec<LinkRow>, IndexError> {
        ctx.authorize(Capability::Read)?;
        let vault = self.engine.vault();
        let mut out = Vec::new();
        for block in vault.blocks() {
            for row in block_links(vault, block) {
                if row.target_id.is_none() {
                    out.push(row);
                }
            }
        }
        Ok(out)
    }

    // ----- writes -----

    /// Create a new block (optional title + body). Returns the new id.
    pub fn create_block(
        &mut self,
        ctx: &RequestContext,
        title: Option<&str>,
        body: &str,
    ) -> Result<BlockId, IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.create_block(title, body)
    }

    /// Overwrite a block's title + body.
    pub fn update_block(
        &mut self,
        ctx: &RequestContext,
        id: &BlockId,
        title: Option<&str>,
        body: &str,
    ) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.update_block(id, title, body)
    }

    /// Set a block's managed (frontmatter) `tags:` to exactly `tags`. Inline `#hashtag`
    /// mentions in the body are untouched; the title and body are preserved.
    pub fn set_tags(
        &mut self,
        ctx: &RequestContext,
        id: &BlockId,
        tags: &[String],
    ) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.set_tags(id, tags)
    }

    /// Delete a block (file + index).
    pub fn delete_block(&mut self, ctx: &RequestContext, id: &BlockId) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.delete_block(id)
    }

    /// Carve a new child block out of an existing block: the new block gets `body`, and a
    /// `![[<newid>]]` directive is appended to the parent in place. Non-destructive: the
    /// parent's other content is untouched. Returns the new child id.
    pub fn carve_block(
        &mut self,
        ctx: &RequestContext,
        parent_id: &BlockId,
        title: Option<&str>,
        body: &str,
    ) -> Result<BlockId, IndexError> {
        ctx.authorize(Capability::Write)?;
        if self.engine.vault().block(parent_id).is_none() {
            return Err(IndexError::new(format!("unknown block: {parent_id}")));
        }
        let child = self.engine.create_block(title, body)?;
        self.engine
            .append_to_body(parent_id, &format!("![[{child}]]"))?;
        Ok(child)
    }

    /// Carve a **selected range** of a parent block's body into a new child block: the text in
    /// `start..end` (byte offsets into the parent's raw body) becomes a new block, and that
    /// exact range is replaced in place by `![[<newid>]]`. This is the "extract a reusable
    /// chunk where it sits" gesture — non-destructive (rendered output is unchanged) and the
    /// content moves into its own addressable block. Returns the new child id.
    pub fn carve_selection(
        &mut self,
        ctx: &RequestContext,
        parent_id: &BlockId,
        start: usize,
        end: usize,
    ) -> Result<BlockId, IndexError> {
        ctx.authorize(Capability::Write)?;
        let parent = self
            .engine
            .vault()
            .block(parent_id)
            .ok_or_else(|| IndexError::new(format!("unknown block: {parent_id}")))?;
        let body = parent.body.clone();
        let title = parent.title.clone();
        if start >= end
            || end > body.len()
            || !body.is_char_boundary(start)
            || !body.is_char_boundary(end)
        {
            return Err(IndexError::new("invalid carve selection range"));
        }
        let selected = body[start..end].trim().to_string();
        if selected.is_empty() {
            return Err(IndexError::new("carve selection is empty"));
        }
        // Create the child first, then splice the parent (so a failed child create leaves the
        // parent untouched).
        let child = self.engine.create_block(None, &selected)?;
        let mut new_body = String::with_capacity(body.len());
        new_body.push_str(&body[..start]);
        new_body.push_str(&format!("![[{child}]]"));
        new_body.push_str(&body[end..]);
        self.engine
            .update_block(parent_id, title.as_deref(), &new_body)?;
        Ok(child)
    }

    /// Link or embed `source_id` to `target_id`. `embed = true` appends `![[target]]`,
    /// `false` appends `[[target]]`.
    ///
    /// If an **embed** would create a transclusion cycle (the target already transcludes its
    /// way back to the source, or is the source itself), it is **downgraded** to a plain
    /// `[[reference]]` — the link is still made, it just won't recurse. The returned
    /// [`LinkOutcome`] reports whether a downgrade happened. References are never cycle-checked.
    pub fn link_blocks(
        &mut self,
        ctx: &RequestContext,
        source_id: &BlockId,
        target_id: &BlockId,
        embed: bool,
    ) -> Result<LinkOutcome, IndexError> {
        ctx.authorize(Capability::Write)?;
        if self.engine.vault().block(source_id).is_none() {
            return Err(IndexError::new(format!(
                "unknown source block: {source_id}"
            )));
        }
        if self.engine.vault().block(target_id).is_none() {
            return Err(IndexError::new(format!(
                "unknown target block: {target_id}"
            )));
        }

        let mut effective_embed = embed;
        if embed
            && (source_id == target_id
                || transclusion_reaches(self.engine.vault(), target_id, source_id))
        {
            effective_embed = false;
        }

        let directive = if effective_embed {
            format!("![[{target_id}]]")
        } else {
            format!("[[{target_id}]]")
        };
        self.engine.append_to_body(source_id, &directive)?;

        Ok(match (embed, effective_embed) {
            (false, _) => LinkOutcome::Reference,
            (true, true) => LinkOutcome::Transclusion,
            (true, false) => LinkOutcome::DowngradedToReference,
        })
    }

    /// Reconcile the `blocks/` directory with the index (startup + watcher).
    pub fn reconcile(&mut self, ctx: &RequestContext) -> Result<SyncReport, IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.reconcile()
    }

    /// Rebuild the entire index from the block files.
    pub fn rebuild(&mut self, ctx: &RequestContext) -> Result<SyncReport, IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.rebuild()
    }

    /// Cloud-sync conflict files detected at the last reconcile.
    pub fn conflicts(&self, ctx: &RequestContext) -> Result<Vec<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(self.engine.conflicts().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::testing::MemIndex;
    use crate::SyncEngine;

    fn service(root: &std::path::Path) -> Service<MemIndex> {
        Service::new(SyncEngine::new(root, MemIndex::default()))
    }

    #[test]
    fn create_update_render_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();

        let id = svc
            .create_block(&ctx, Some("Note"), "original body")
            .unwrap();
        assert_eq!(
            svc.get_block(&ctx, &id).unwrap().unwrap().content,
            "original body"
        );
        svc.update_block(&ctx, &id, Some("Note"), "updated body")
            .unwrap();
        assert_eq!(
            svc.get_block_source(&ctx, &id).unwrap().unwrap(),
            "updated body"
        );
    }

    #[test]
    fn carve_creates_child_and_links_parent() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let parent = svc.create_block(&ctx, Some("Guide"), "intro").unwrap();
        let child = svc
            .carve_block(&ctx, &parent, Some("Shared step"), "do the thing")
            .unwrap();
        // Parent now embeds the child; rendering pulls the child content.
        let rendered = svc.render_block(&ctx, &parent).unwrap().unwrap();
        assert!(rendered.contains("do the thing"), "got: {rendered}");
        assert!(svc.engine().vault().children(&parent).contains(&child));
    }

    #[test]
    fn carve_selection_extracts_range_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let body = "before SHARED after";
        let parent = svc.create_block(&ctx, Some("Guide"), body).unwrap();
        let start = body.find("SHARED").unwrap();
        let end = start + "SHARED".len();
        let child = svc.carve_selection(&ctx, &parent, start, end).unwrap();

        // The carved text became its own block...
        assert_eq!(
            svc.get_block(&ctx, &child).unwrap().unwrap().content,
            "SHARED"
        );
        // ...replaced in place by an embed, so the parent body reads "before ![[id]] after".
        let psrc = svc.get_block_source(&ctx, &parent).unwrap().unwrap();
        assert_eq!(psrc, format!("before ![[{child}]] after"));
        // Rendered output is unchanged (the child is inlined where it was).
        let rendered = svc.render_block(&ctx, &parent).unwrap().unwrap();
        assert!(rendered.contains("SHARED"));
        assert!(svc.engine().vault().children(&parent).contains(&child));
    }

    #[test]
    fn carve_selection_rejects_bad_range() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let parent = svc.create_block(&ctx, None, "short").unwrap();
        assert!(svc.carve_selection(&ctx, &parent, 3, 3).is_err()); // empty
        assert!(svc.carve_selection(&ctx, &parent, 0, 999).is_err()); // out of range
    }

    #[test]
    fn link_embed_reflects_in_render() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let q = svc
            .create_block(&ctx, Some("Q"), "StormEvents | take 10")
            .unwrap();
        let host = svc
            .create_block(&ctx, Some("Host"), "Project notes:")
            .unwrap();
        assert_eq!(
            svc.link_blocks(&ctx, &host, &q, true).unwrap(),
            LinkOutcome::Transclusion
        );
        let rendered = svc.render_block(&ctx, &host).unwrap().unwrap();
        assert!(rendered.contains("StormEvents | take 10"));
    }

    #[test]
    fn embed_that_would_cycle_is_downgraded() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let a = svc.create_block(&ctx, Some("A"), "block a").unwrap();
        let b = svc.create_block(&ctx, Some("B"), "block b").unwrap();
        // A embeds B.
        assert_eq!(
            svc.link_blocks(&ctx, &a, &b, true).unwrap(),
            LinkOutcome::Transclusion
        );
        // B embedding A would cycle -> downgraded to a reference.
        assert_eq!(
            svc.link_blocks(&ctx, &b, &a, true).unwrap(),
            LinkOutcome::DowngradedToReference
        );
        let b_src = svc.get_block_source(&ctx, &b).unwrap().unwrap();
        assert!(b_src.contains(&format!("[[{a}]]")));
        assert!(!b_src.contains(&format!("![[{a}]]")));
        // Render still terminates.
        assert!(svc.render_block(&ctx, &a).unwrap().is_some());
        assert!(svc.render_block(&ctx, &b).unwrap().is_some());
    }

    #[test]
    fn self_embed_is_downgraded() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let a = svc.create_block(&ctx, None, "note").unwrap();
        assert_eq!(
            svc.link_blocks(&ctx, &a, &a, true).unwrap(),
            LinkOutcome::DowngradedToReference
        );
    }

    #[test]
    fn reference_cycle_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let a = svc.create_block(&ctx, None, "A").unwrap();
        let b = svc.create_block(&ctx, None, "B").unwrap();
        assert_eq!(
            svc.link_blocks(&ctx, &a, &b, false).unwrap(),
            LinkOutcome::Reference
        );
        assert_eq!(
            svc.link_blocks(&ctx, &b, &a, false).unwrap(),
            LinkOutcome::Reference
        );
    }

    #[test]
    fn remote_caller_is_denied_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path());
        let ctx = RequestContext::remote("agent-7");
        assert!(
            svc.list_blocks(&ctx).is_err(),
            "remote callers must fail closed"
        );
    }

    #[test]
    fn delete_removes_block() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let id = svc.create_block(&ctx, None, "x").unwrap();
        assert_eq!(svc.stats(&ctx).unwrap().blocks, 1);
        svc.delete_block(&ctx, &id).unwrap();
        assert_eq!(svc.stats(&ctx).unwrap().blocks, 0);
    }
}
