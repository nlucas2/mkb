//! The mkb service API: the single, shared surface every client uses.
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
    block_links, group_blocks_by, hierarchy_tree, link_graph, transclusion_reaches, BlockRecord,
    GraphData, GroupAxis, GroupTree, HierTree, Index, IndexError, IndexStats, LinkCrumb,
    LinkOutcome, LinkRow, PageView, SearchHit, SearchQuery, TagCount,
};
use crate::link::extract_references;
use crate::render::{
    current_line_width, reindent_continuation, render_block, render_flat, rendered_block,
    RenderedBlock,
};
use crate::sync::{SyncEngine, SyncReport};

/// The result of a [`Service::update_block`] attempt under optimistic concurrency.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum UpdateOutcome {
    /// The write was applied. Carries the block's new version (so a client can keep editing
    /// against fresh state without a re-read).
    Applied {
        /// The post-write content version.
        version: String,
    },
    /// The write was **rejected**: the caller's `base_version` no longer matches the daemon's
    /// current version, so applying the full-body overwrite would have clobbered a change made
    /// since the caller last read the block. Carries the current state so a client can show a
    /// conflict/merge UI without a second request. Nothing was written.
    Conflict {
        /// The block's current title.
        current_title: Option<String>,
        /// The block's current body.
        current_body: String,
        /// The block's current version (what `base_version` must equal to apply).
        version: String,
    },
}

/// Format a raw content hash as the opaque version token clients echo back. Hex of the `u64` the
/// [`SyncEngine`] already maintains; clients treat it as opaque (never parse it), so the underlying
/// representation can change without a protocol break.
fn format_version(hash: u64) -> String {
    format!("{hash:016x}")
}

/// A block is at least this many characters before the "drastic shrink" guard applies, so trimming
/// a small note is never blocked — only substantial blocks are protected from mass deletion.
const UPDATE_GUARD_MIN_CHARS: usize = 200;
/// An update that removes more than this fraction of a substantial block's content is treated as
/// destructive and refused unless forced.
const UPDATE_GUARD_MAX_LOSS: f64 = 0.75;

/// Whether replacing `old` body with `new` would destroy content an agent likely didn't mean to
/// lose, returning a human-readable reason if so. Two cases: emptying a non-empty block, or
/// stripping more than [`UPDATE_GUARD_MAX_LOSS`] of a block of at least [`UPDATE_GUARD_MIN_CHARS`].
/// Comparison is on trimmed character counts, so reformatting (similar length) never trips it; only
/// truncation does. This is the heuristic behind [`Service::update_block`]'s force gate.
pub fn destructive_update_reason(old: &str, new: &str) -> Option<String> {
    let old_t = old.trim();
    if old_t.is_empty() {
        return None;
    }
    if new.trim().is_empty() {
        return Some("the new body is empty but the block is not".to_string());
    }
    let old_len = old_t.chars().count();
    let new_len = new.trim().chars().count();
    if old_len >= UPDATE_GUARD_MIN_CHARS && new_len < old_len {
        let lost = (old_len - new_len) as f64 / old_len as f64;
        if lost > UPDATE_GUARD_MAX_LOSS {
            return Some(format!(
                "it removes about {:.0}% of the block's content ({old_len} → {new_len} chars)",
                lost * 100.0
            ));
        }
    }
    None
}

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

/// A capability a request needs. The set a caller holds is its [`Scope`]; `authorize` checks
/// membership. This is the seam for scoped authentication — e.g. a read-only token grants only
/// `Read`, and `ManageLocks` (lock/unlock) is reserved for the desktop app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Read-only access (search, get, render, graph…).
    Read,
    /// Mutate an **unlocked** block (create/update/set-tags/delete/carve/flatten/link).
    Write,
    /// Lock or unlock a block (toggle the human-only flag). App-only.
    ManageLocks,
}

/// The set of capabilities a caller has been granted — a small, explicit authorization scope.
/// Locked-block writes are governed by the block's own state (a locked block is immutable to
/// *every* scope via the write path); `ManageLocks` gates the unlock that precedes an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    read: bool,
    write: bool,
    manage_locks: bool,
}

impl Scope {
    /// No access — un-authenticated remote callers fail closed.
    pub const NONE: Scope = Scope {
        read: false,
        write: false,
        manage_locks: false,
    };
    /// Read but not write (the shape a future read-only token would grant).
    pub const READ_ONLY: Scope = Scope {
        read: true,
        write: false,
        manage_locks: false,
    };
    /// Read + write unlocked blocks, but **cannot** lock/unlock. The default for machine clients
    /// (CLI, MCP) and token-authenticated callers today.
    pub const AGENT: Scope = Scope {
        read: true,
        write: true,
        manage_locks: false,
    };
    /// Everything, including lock management. Granted to the desktop app (the human surface).
    pub const APP: Scope = Scope {
        read: true,
        write: true,
        manage_locks: true,
    };

    /// Whether this scope grants `cap`.
    pub fn can(&self, cap: Capability) -> bool {
        match cap {
            Capability::Read => self.read,
            Capability::Write => self.write,
            Capability::ManageLocks => self.manage_locks,
        }
    }
}

/// Per-request context: who is calling (identity) and what they may do (scope).
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// The caller (transport identity, for logging/diagnostics).
    pub caller: Caller,
    /// The granted capability scope.
    pub scope: Scope,
}

impl RequestContext {
    /// A local, trusted context with the **agent** scope (read + write, no lock management). This
    /// is the default for the CLI and MCP server; the desktop app upgrades to [`Scope::APP`].
    pub fn local() -> Self {
        RequestContext {
            caller: Caller::Local,
            scope: Scope::AGENT,
        }
    }

    /// A local context with the full **app** scope (adds lock management). Used by the desktop
    /// app — the single human surface allowed to lock/unlock.
    pub fn local_app() -> Self {
        RequestContext {
            caller: Caller::Local,
            scope: Scope::APP,
        }
    }

    /// A remote context with an opaque principal id. Fails closed (empty scope) until it
    /// authenticates with a token.
    pub fn remote(id: impl Into<String>) -> Self {
        RequestContext {
            caller: Caller::Remote(id.into()),
            scope: Scope::NONE,
        }
    }

    /// A network context that has presented a valid shared token: the **agent** scope today
    /// (read + write, no lock management). Scoped tokens (e.g. read-only) plug in here.
    pub fn authenticated(id: impl Into<String>) -> Self {
        RequestContext {
            caller: Caller::Authenticated(id.into()),
            scope: Scope::AGENT,
        }
    }

    /// Authorize `cap` against this context's scope. The single choke point for authorization.
    pub fn authorize(&self, cap: Capability) -> Result<(), IndexError> {
        if self.scope.can(cap) {
            return Ok(());
        }
        let msg = match (&self.caller, cap) {
            (Caller::Remote(id), _) => {
                format!("unauthorized: remote caller {id} (authenticate with a token first)")
            }
            (_, Capability::ManageLocks) => {
                "locking/unlocking is human-only: do it in the desktop app".to_string()
            }
            (_, c) => format!("unauthorized: missing {c:?} capability"),
        };
        Err(IndexError::new(msg))
    }
}

/// The mkb service: wraps a [`SyncEngine`] and exposes the deterministic block API.
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
        // Date and property (has/missing) filters are applied here, not in the index — created from
        // the id, updated/props overlaid from the vault. So when one is present, fetch *all*
        // candidates from the engine (bounded by the vault block count, since it can't return more
        // than that), then filter and re-apply the real limit. `0` would not work: the engine maps
        // it to the default 50 via `effective_limit`, which would silently drop matches past the
        // 50th — exactly the "blocks missing X across the whole vault" audit.
        let post_filter = query.has_date_filter() || query.has_prop_filter();
        let limit = query.effective_limit();
        let mut engine_query = query.clone();
        if post_filter {
            engine_query.limit = self.engine.vault().len().max(1);
        }
        let mut hits = self.engine.search(engine_query)?;
        for hit in &mut hits {
            self.overlay_metadata(&mut hit.block);
        }
        if post_filter {
            hits.retain(|h| query.matches_dates(&h.block) && query.matches_props(&h.block));
            hits.truncate(limit);
        }
        // Annotate each hit with its upward lineage (which page(s) it lives on) so a hit on a
        // nested, embedded block isn't a context-free fragment. Build the reverse-edge map once.
        let parents = self.engine.vault().embed_parent_map();
        for hit in &mut hits {
            hit.lineage = Some(self.engine.vault().lineage_with(&parents, &hit.block.id));
        }
        Ok(hits)
    }

    /// Fill in the metadata the index doesn't persist: `locked`/`props`/`updated` from the parsed
    /// vault, and `created` decoded from the block's ULID id. `created` is set even if the block is
    /// (transiently) absent from the vault, since it depends only on the id.
    fn overlay_metadata(&self, rec: &mut BlockRecord) {
        rec.created = rec.id.created_rfc3339();
        if let Some(b) = self.engine.vault().block(&rec.id) {
            rec.locked = b.locked;
            rec.props = b.props.clone();
            rec.updated = b.updated.clone();
        }
    }

    /// Fetch a single block record by id.
    pub fn get_block(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
    ) -> Result<Option<BlockRecord>, IndexError> {
        ctx.authorize(Capability::Read)?;
        let mut record = self.engine.index().block(id)?;
        // The index doesn't persist `locked`/`props`/`updated` (they live in the file frontmatter)
        // or `created` (decoded from the id); overlay the authoritative values so clients see the
        // true lock state, the block's open-ended metadata, and its timestamps.
        if let Some(rec) = record.as_mut() {
            self.overlay_metadata(rec);
        }
        Ok(record)
    }

    /// Display title for a neighbour block (falls back to its id when untitled/absent).
    fn crumb_title(&self, id: &BlockId) -> String {
        self.engine
            .vault()
            .block(id)
            .map(|b| b.display_title())
            .unwrap_or_else(|| id.as_str().to_string())
    }

    /// A rich, self-contained read of a block: its record plus where it lives (`lineage`) and its
    /// direct relationships in both directions. This is the one read that answers "show me this
    /// block and everything around it", folding together [`get_block`], [`render_block`],
    /// [`block_source_range`], [`backlinks`], and [`links_from`].
    ///
    /// The returned `block.content` reflects the requested view of the body:
    /// - `rendered` → transclusions resolved (children inlined);
    /// - else `start`/`end` (1-based, inclusive) → that line range only;
    /// - else the raw body verbatim.
    ///
    /// [`get_block`]: Service::get_block
    /// [`render_block`]: Service::render_block
    /// [`block_source_range`]: Service::block_source_range
    /// [`backlinks`]: Service::backlinks
    /// [`links_from`]: Service::links_from
    pub fn page_view(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
        rendered: bool,
        start: Option<usize>,
        end: Option<usize>,
    ) -> Result<Option<PageView>, IndexError> {
        ctx.authorize(Capability::Read)?;
        let mut block = match self.engine.index().block(id)? {
            Some(r) => r,
            None => return Ok(None),
        };
        self.overlay_metadata(&mut block);
        // Shape the body to the requested view, reusing the same primitives the dedicated
        // read tools use so behavior can never diverge.
        if rendered {
            if let Some(r) = render_block(self.engine.vault(), id) {
                block.content = r;
            }
        } else if start.is_some() || end.is_some() {
            let s = start.unwrap_or(1);
            let e = end.unwrap_or(usize::MAX);
            block.content = crate::slice_lines(&block.content, s, e);
        }
        let lineage = self.engine.vault().lineage(id);
        let backlinks = self
            .engine
            .index()
            .backlinks(id)?
            .into_iter()
            .map(|r| LinkCrumb {
                title: self.crumb_title(&r.source_id),
                id: r.source_id,
                kind: r.kind,
            })
            .collect();
        let links_out = self
            .engine
            .index()
            .links_from(id)?
            .into_iter()
            .filter_map(|r| {
                r.target_id.map(|t| LinkCrumb {
                    title: self.crumb_title(&t),
                    id: t,
                    kind: r.kind,
                })
            })
            .collect();
        Ok(Some(PageView {
            block,
            lineage,
            backlinks,
            links_out,
        }))
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

    /// Group the vault's blocks by an axis (tags or an arbitrary property key) into a `/`-nested
    /// tree, for the sidebar's group-by view. A pure read projection — no writes, no validation.
    pub fn group_blocks_by(
        &self,
        ctx: &RequestContext,
        axis: &GroupAxis,
    ) -> Result<GroupTree, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(group_blocks_by(self.engine.vault(), axis))
    }

    /// The composition hierarchy: roots, each expandable into the blocks it embeds/links. A pure
    /// read projection — no writes.
    pub fn hierarchy(&self, ctx: &RequestContext) -> Result<HierTree, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(hierarchy_tree(self.engine.vault()))
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

    /// Guard a mutation against the human-only (`locked`) rule: a locked block is **immutable to
    /// every caller** via the write path — there is no write-through. To change it, a human
    /// unlocks it in the app first (see [`Service::set_lock`]), edits, then re-locks. Returns `Ok`
    /// if the block is unlocked or absent. This is the single enforcement point reused by every
    /// write that targets an existing block (update/set-tags/delete/carve-from/flatten/link-into).
    fn ensure_writable(&self, id: &BlockId) -> Result<(), IndexError> {
        if let Some(b) = self.engine.vault().block(id) {
            if b.locked {
                return Err(IndexError::new(format!(
                    "block {id} is locked (human-only): unlock it in the desktop app first"
                )));
            }
        }
        Ok(())
    }

    /// Lock or unlock a block (the human-only flag). Requires the [`Capability::ManageLocks`]
    /// capability, which only the desktop app's scope holds — so lock state can only be toggled
    /// from the app (or by a human editing the file's `locked:` frontmatter directly).
    pub fn set_lock(
        &mut self,
        ctx: &RequestContext,
        id: &BlockId,
        locked: bool,
    ) -> Result<(), IndexError> {
        ctx.authorize(Capability::ManageLocks)?;
        self.engine.set_lock(id, locked)
    }

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

    /// List orphaned assets — files under `assets/` referenced by no block. Read-only.
    pub fn orphan_assets(&self, ctx: &RequestContext) -> Result<Vec<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(self.engine.orphan_assets())
    }

    /// Delete an asset by its vault-relative `assets/…` path. Requires [`Capability::Write`].
    pub fn remove_asset(&self, ctx: &RequestContext, rel: &str) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.remove_asset(rel)
    }

    /// Import a binary asset (image, etc.) into the vault's `assets/` directory under a safe,
    /// unique filename, returning the vault-relative path to reference from a block. Requires the
    /// [`Capability::Write`] capability. Assets are not indexed.
    pub fn add_asset(
        &self,
        ctx: &RequestContext,
        name: &str,
        bytes: &[u8],
    ) -> Result<String, IndexError> {
        ctx.authorize(Capability::Write)?;
        self.engine.add_asset(name, bytes)
    }

    /// The current content version of a block (opaque optimistic-concurrency token), or `None` if
    /// the block is unknown. A client captures this when it opens a block for editing and passes it
    /// back to [`update_block`](Service::update_block) as `base_version`.
    pub fn block_version(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
    ) -> Result<Option<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(self.engine.block_version(id).map(format_version))
    }

    /// Overwrite a block's title + body.
    ///
    /// `update_block` replaces the **entire** body, so a caller that sends a truncated or empty
    /// body silently destroys the block's content. To make that hard to do by accident, the update
    /// is **refused** when it would empty a block or strip most of its content (see
    /// [`destructive_update_reason`]) — unless `force` is set, which an intentional rewrite passes.
    /// The guard lives here, at the user-facing op; the internal structural rewrites
    /// (`carve_selection`/`flatten_block`) go straight to the engine and are never guarded.
    ///
    /// Under optimistic concurrency, if `base_version` is `Some`, the write is **rejected with a
    /// [`UpdateOutcome::Conflict`]** (and nothing is written) when it no longer matches the block's
    /// current version — i.e. the block changed since the caller read it, so a blind overwrite
    /// would clobber that change. `None` skips the check (the historical behaviour, for
    /// non-interactive callers). `force` is independent: it bypasses the destructive-shrink guard,
    /// not the concurrency check.
    pub fn update_block(
        &mut self,
        ctx: &RequestContext,
        id: &BlockId,
        title: Option<&str>,
        body: &str,
        force: bool,
        base_version: Option<&str>,
    ) -> Result<UpdateOutcome, IndexError> {
        ctx.authorize(Capability::Write)?;
        self.ensure_writable(id)?;
        // Optimistic-concurrency check FIRST: if the caller pinned a base version and it no longer
        // matches, refuse and hand back the current state so the client can reconcile. Compared
        // against the daemon's in-memory version (it is the single writer, so that is authoritative
        // for every write applied through it).
        if let Some(base) = base_version {
            let current = self.engine.block_version(id).map(format_version);
            if current.as_deref() != Some(base) {
                if let Some(rec) = self.engine.index().block(id)? {
                    return Ok(UpdateOutcome::Conflict {
                        current_title: self.engine.vault().block(id).and_then(|b| b.title.clone()),
                        current_body: rec.content,
                        version: current.unwrap_or_default(),
                    });
                }
                // The block vanished entirely since the read — treat as a conflict with no content.
                return Ok(UpdateOutcome::Conflict {
                    current_title: None,
                    current_body: String::new(),
                    version: String::new(),
                });
            }
        }
        if !force {
            if let Some(old) = self.engine.vault().block(id) {
                if let Some(reason) = destructive_update_reason(&old.body, body) {
                    return Err(IndexError::new(format!(
                        "refusing to update {id}: {reason}. If this is an intentional rewrite, \
                         retry with force; otherwise read the current body first (get) and send \
                         the full revised text — update replaces the whole body."
                    )));
                }
            }
        }
        self.engine.update_block(id, title, body)?;
        let version = self
            .engine
            .block_version(id)
            .map(format_version)
            .unwrap_or_default();
        Ok(UpdateOutcome::Applied { version })
    }

    /// Apply an exact, count-checked string replacement to a block's **body** — the partial-edit
    /// primitive (the others all replace the whole body). `old` must occur exactly `expect_count`
    /// times in the raw body or the call errors and nothing is written, so an ambiguous or stale
    /// anchor is a safe no-op. The resulting body still passes through the destructive-update guard
    /// (unless `force`), preserves title/tags/lock/props, and stamps `updated:` like any other edit.
    pub fn replace_in_block(
        &mut self,
        ctx: &RequestContext,
        id: &BlockId,
        old: &str,
        new: &str,
        expect_count: usize,
        force: bool,
    ) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        self.ensure_writable(id)?;
        let old_body = self
            .engine
            .vault()
            .block(id)
            .ok_or_else(|| IndexError::new(format!("unknown block: {id}")))?
            .body
            .clone();
        let new_body = crate::exact_replace(&old_body, old, new, expect_count)?;
        if new_body == old_body {
            return Ok(());
        }
        if !force {
            if let Some(reason) = destructive_update_reason(&old_body, &new_body) {
                return Err(IndexError::new(format!(
                    "refusing to edit {id}: {reason}. If this is an intentional rewrite, retry \
                     with force."
                )));
            }
        }
        // Title None preserves the existing title (and tags/lock/props are kept by update_block).
        self.engine.update_block(id, None, &new_body)
    }

    /// Append `text` to a block's body (it starts on a fresh line). Purely additive — it never
    /// removes content, so the destructive guard doesn't apply — but it is still a write: it
    /// requires the [`Capability::Write`] capability, is refused on a locked block, preserves
    /// title/tags/lock/props, and stamps `updated:`.
    pub fn append_to_block(
        &mut self,
        ctx: &RequestContext,
        id: &BlockId,
        text: &str,
    ) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        self.ensure_writable(id)?;
        let old_body = self
            .engine
            .vault()
            .block(id)
            .ok_or_else(|| IndexError::new(format!("unknown block: {id}")))?
            .body
            .clone();
        let new_body = crate::append_text(&old_body, text);
        self.engine.update_block(id, None, &new_body)
    }

    /// Return lines `start..=end` (1-based, inclusive) of a block's raw source body, or `None` if
    /// the block doesn't exist. A read-only convenience for viewing a slice of a large block.
    pub fn block_source_range(
        &self,
        ctx: &RequestContext,
        id: &BlockId,
        start: usize,
        end: usize,
    ) -> Result<Option<String>, IndexError> {
        ctx.authorize(Capability::Read)?;
        Ok(self
            .engine
            .vault()
            .block(id)
            .map(|b| crate::slice_lines(&b.body, start, end)))
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
        self.ensure_writable(id)?;
        self.engine.set_tags(id, tags)
    }

    /// **Merge** properties into a block: each `(key, value)` is added or updates that key, and
    /// every other property is preserved (add/update-only — no operation replaces the whole set).
    /// Open-ended `key: value` metadata; title, tags, lock state, and body are untouched. Requires
    /// `Write` and the block must be unlocked.
    pub fn set_props(
        &mut self,
        ctx: &RequestContext,
        id: &BlockId,
        props: &[(String, String)],
    ) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        self.ensure_writable(id)?;
        self.engine.set_props(id, props)
    }

    /// Remove the named properties from a block, preserving all other properties (and title, tags,
    /// lock state, body). Unknown keys are ignored. Requires `Write` and the block must be unlocked.
    pub fn unset_props(
        &mut self,
        ctx: &RequestContext,
        id: &BlockId,
        keys: &[String],
    ) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        self.ensure_writable(id)?;
        self.engine.unset_props(id, keys)
    }

    /// Delete a block (file + index).
    pub fn delete_block(&mut self, ctx: &RequestContext, id: &BlockId) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        self.ensure_writable(id)?;
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
        self.ensure_writable(parent_id)?;
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
        self.ensure_writable(parent_id)?;
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

    /// **Flatten / uncarve** — the inverse of carve. Inline the `![[child]]` embed in `parent`
    /// back into the parent's body (the child's raw content replaces the directive in place,
    /// re-indented to the call-site column) and **delete** the now-redundant child block.
    ///
    /// **Strict single-reference semantic.** This only applies when `child` is referenced in
    /// **exactly one** place in the whole vault — a single `![[child]]` embed, located in
    /// `parent`, with no other embedders and no `[[references]]`. If the child is used anywhere
    /// else (another embed, the parent embedding it twice, or any reference), flatten **errors**
    /// and changes nothing — it never partially inlines a block that is still used elsewhere.
    ///
    /// One level only: the child's own `![[grandchild]]` directives come along verbatim and stay
    /// their own blocks. Cycles are a non-issue (embeds can't cycle). The child's removal is safe
    /// precisely because it had a single embedder, which this call rewrites.
    pub fn flatten_block(
        &mut self,
        ctx: &RequestContext,
        parent_id: &BlockId,
        child_id: &BlockId,
    ) -> Result<(), IndexError> {
        ctx.authorize(Capability::Write)?;
        // Flatten mutates the parent (inlines the embed) and deletes the child, so both must be
        // writable by this caller.
        self.ensure_writable(parent_id)?;
        self.ensure_writable(child_id)?;

        // Read phase: validate, count every directive occurrence targeting the child across the
        // vault, and capture the single occurrence's span (so the immutable borrow ends before we
        // mutate). Counting is by directive *occurrence*, so a parent embedding the child twice is
        // two references and (correctly) rejected.
        let (parent_body, parent_title, child_body, span) = {
            let vault = self.engine.vault();
            let parent = vault
                .block(parent_id)
                .ok_or_else(|| IndexError::new(format!("unknown block: {parent_id}")))?;
            let child = vault
                .block(child_id)
                .ok_or_else(|| IndexError::new(format!("unknown block: {child_id}")))?;

            let mut total = 0usize;
            let mut here: Option<(bool, std::ops::Range<usize>)> = None;
            for b in vault.blocks() {
                for r in extract_references(&b.body) {
                    if vault.resolve(&r.target).as_ref() == Some(child_id) {
                        total += 1;
                        if &b.id == parent_id {
                            here = Some((r.embed, r.span.clone()));
                        }
                    }
                }
            }

            if total != 1 {
                return Err(IndexError::new(format!(
                    "cannot flatten {child_id}: it is referenced in {total} place(s); \
                     flatten requires exactly one (a single ![[…]] embed in the parent)"
                )));
            }
            let (embed, span) = here.ok_or_else(|| {
                IndexError::new(format!(
                    "cannot flatten {child_id}: its only reference is not in {parent_id}"
                ))
            })?;
            if !embed {
                return Err(IndexError::new(format!(
                    "cannot flatten {child_id}: its single reference is a [[reference]], \
                     not a ![[embed]]"
                )));
            }
            (
                parent.body.clone(),
                parent.title.clone(),
                child.body.clone(),
                span,
            )
        };

        // Splice the child's raw body in place of the directive, re-indented to the call-site
        // column (so an embed inside a list item / YAML scalar stays well-formed; column 0 is a
        // no-op). Surrounding blank lines on the child are trimmed so no extra spacing creeps in.
        let col = current_line_width(&parent_body[..span.start]);
        let inlined = reindent_continuation(child_body.trim_matches('\n'), col);
        let mut new_body = String::with_capacity(parent_body.len() + inlined.len());
        new_body.push_str(&parent_body[..span.start]);
        new_body.push_str(&inlined);
        new_body.push_str(&parent_body[span.end..]);

        // Apply: rewrite the parent (dropping its embed of the child), then delete the orphan.
        self.engine
            .update_block(parent_id, parent_title.as_deref(), &new_body)?;
        self.engine.delete_block(child_id)?;
        Ok(())
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
        // Linking appends a directive to the source block, so the source must be writable.
        self.ensure_writable(source_id)?;
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
    fn destructive_update_reason_flags_emptying_and_mass_deletion() {
        // Emptying a non-empty block is always flagged.
        assert!(destructive_update_reason("some real content", "   ").is_some());
        // Emptying an already-empty block is fine.
        assert!(destructive_update_reason("  ", "").is_none());
        // A normal edit on a substantial block is fine.
        let big = "x".repeat(400);
        assert!(destructive_update_reason(&big, &"x".repeat(380)).is_none());
        // Stripping most of a substantial block is flagged.
        assert!(destructive_update_reason(&big, "tiny").is_some());
        // A small block can be trimmed freely (below the guard's size floor).
        assert!(destructive_update_reason("short note", "s").is_none());
    }

    #[test]
    fn update_block_guard_blocks_truncation_unless_forced() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let big = "paragraph of real content. ".repeat(20); // ~540 chars
        let id = svc.create_block(&ctx, Some("Doc"), &big).unwrap();

        // An accidental truncation is refused and changes nothing.
        let err = svc.update_block(&ctx, &id, Some("Doc"), "oops", false, None);
        assert!(err.is_err(), "truncating update should be refused");
        assert_eq!(svc.get_block_source(&ctx, &id).unwrap().unwrap(), big);

        // Emptying is refused too.
        assert!(svc
            .update_block(&ctx, &id, Some("Doc"), "", false, None)
            .is_err());

        // The same edit goes through when explicitly forced (a deliberate rewrite).
        svc.update_block(&ctx, &id, Some("Doc"), "oops", true, None)
            .unwrap();
        assert_eq!(svc.get_block_source(&ctx, &id).unwrap().unwrap(), "oops");
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
        svc.update_block(&ctx, &id, Some("Note"), "updated body", false, None)
            .unwrap();
        assert_eq!(
            svc.get_block_source(&ctx, &id).unwrap().unwrap(),
            "updated body"
        );
    }

    #[test]
    fn update_block_optimistic_concurrency() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let id = svc
            .create_block(
                &ctx,
                Some("Doc"),
                "the original body, long enough to matter",
            )
            .unwrap();

        // A read captures the current version.
        let v0 = svc
            .block_version(&ctx, &id)
            .unwrap()
            .expect("has a version");

        // Updating with the matching base version applies and reports the NEW version.
        let outcome = svc
            .update_block(&ctx, &id, None, "first revision body", false, Some(&v0))
            .unwrap();
        let v1 = match outcome {
            UpdateOutcome::Applied { version } => version,
            UpdateOutcome::Conflict { .. } => panic!("matching base must apply, not conflict"),
        };
        assert_ne!(v0, v1, "the version must move after a write");
        assert_eq!(
            svc.get_block_source(&ctx, &id).unwrap().unwrap(),
            "first revision body"
        );

        // A second write that still pins the STALE v0 is rejected as a conflict — nothing written,
        // and the current state is handed back for the client to reconcile.
        match svc
            .update_block(&ctx, &id, None, "clobbering body", true, Some(&v0))
            .unwrap()
        {
            UpdateOutcome::Conflict {
                current_body,
                version,
                ..
            } => {
                assert_eq!(current_body, "first revision body");
                assert_eq!(version, v1, "conflict reports the true current version");
            }
            UpdateOutcome::Applied { .. } => panic!("stale base must conflict, not apply"),
        }
        // The rejected write left the body untouched.
        assert_eq!(
            svc.get_block_source(&ctx, &id).unwrap().unwrap(),
            "first revision body"
        );

        // `None` base version skips the check entirely (non-interactive callers).
        svc.update_block(&ctx, &id, None, "unchecked overwrite", true, None)
            .unwrap();
        assert_eq!(
            svc.get_block_source(&ctx, &id).unwrap().unwrap(),
            "unchecked overwrite"
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
    fn flatten_inlines_single_use_child_and_deletes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        // Carve a chunk out, then flatten it back — the round trip restores the in-place content.
        let body = "before SHARED after";
        let parent = svc.create_block(&ctx, Some("Guide"), body).unwrap();
        let start = body.find("SHARED").unwrap();
        let child = svc
            .carve_selection(&ctx, &parent, start, start + "SHARED".len())
            .unwrap();
        assert_eq!(
            svc.get_block_source(&ctx, &parent).unwrap().unwrap(),
            format!("before ![[{child}]] after")
        );

        svc.flatten_block(&ctx, &parent, &child).unwrap();

        // Content is back inline, the embed is gone, and the child block no longer exists.
        assert_eq!(
            svc.get_block_source(&ctx, &parent).unwrap().unwrap(),
            "before SHARED after"
        );
        assert!(svc.get_block(&ctx, &child).unwrap().is_none());
    }

    #[test]
    fn flatten_errors_when_child_has_multiple_embedders() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let child = svc
            .create_block(&ctx, Some("Shared"), "shared body")
            .unwrap();
        let a = svc
            .create_block(&ctx, Some("A"), &format!("a ![[{child}]]"))
            .unwrap();
        let _b = svc
            .create_block(&ctx, Some("B"), &format!("b ![[{child}]]"))
            .unwrap();
        // Two embedders → not flattenable; nothing changes.
        assert!(svc.flatten_block(&ctx, &a, &child).is_err());
        assert!(svc.get_block(&ctx, &child).unwrap().is_some());
        assert!(svc.engine().vault().children(&a).contains(&child));
    }

    #[test]
    fn flatten_errors_when_child_also_referenced() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let child = svc
            .create_block(&ctx, Some("Shared"), "shared body")
            .unwrap();
        let parent = svc
            .create_block(&ctx, Some("Parent"), &format!("p ![[{child}]]"))
            .unwrap();
        // A second block *references* the child → total references = 2 → error.
        let _ref = svc
            .create_block(&ctx, Some("Linker"), &format!("see [[{child}]]"))
            .unwrap();
        assert!(svc.flatten_block(&ctx, &parent, &child).is_err());
        assert!(svc.get_block(&ctx, &child).unwrap().is_some());
    }

    #[test]
    fn flatten_errors_when_single_reference_is_not_an_embed() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let child = svc.create_block(&ctx, Some("Target"), "body").unwrap();
        let parent = svc
            .create_block(&ctx, Some("Parent"), &format!("see [[{child}]]"))
            .unwrap();
        // The lone reference is a [[ref]], not a ![[embed]] → not flattenable.
        assert!(svc.flatten_block(&ctx, &parent, &child).is_err());
        assert!(svc.get_block(&ctx, &child).unwrap().is_some());
    }

    #[test]
    fn flatten_preserves_grandchild_embeds_and_reindents() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let grand = svc.create_block(&ctx, Some("Grand"), "grand body").unwrap();
        // The child embeds a grandchild; the child is embedded once, inside a list item.
        let child = svc
            .create_block(&ctx, Some("Mid"), &format!("first line\n![[{grand}]]"))
            .unwrap();
        let parent = svc
            .create_block(&ctx, Some("Parent"), &format!("- item ![[{child}]]"))
            .unwrap();

        svc.flatten_block(&ctx, &parent, &child).unwrap();

        let src = svc.get_block_source(&ctx, &parent).unwrap().unwrap();
        // Child gone; its grandchild embed survives in the parent (one level only).
        assert!(svc.get_block(&ctx, &child).unwrap().is_none());
        assert!(src.contains(&format!("![[{grand}]]")), "got: {src}");
        // Continuation line re-indented to the call-site column (under the list item).
        assert!(src.contains("- item first line\n"), "got: {src}");
        assert!(svc.engine().vault().children(&parent).contains(&grand));
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
    fn page_view_folds_content_lineage_and_relationships() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let child = svc
            .create_block(&ctx, Some("Child"), "line one\nline two\nline three")
            .unwrap();
        let parent = svc.create_block(&ctx, Some("Parent"), "Parent:").unwrap();
        let sibling = svc.create_block(&ctx, Some("See also"), "ref:").unwrap();
        // parent ![[child]] (embed), sibling [[child]] (reference).
        svc.link_blocks(&ctx, &parent, &child, true).unwrap();
        svc.link_blocks(&ctx, &sibling, &child, false).unwrap();

        // Default: raw body + lineage + both backlink kinds, no outgoing.
        let pv = svc
            .page_view(&ctx, &child, false, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(pv.block.content, "line one\nline two\nline three");
        assert!(!pv.lineage.is_root, "child is embedded, so not a root");
        let roots: Vec<_> = pv.lineage.roots.iter().map(|c| c.id.clone()).collect();
        assert!(roots.contains(&parent), "parent is the embed root");
        assert!(pv
            .backlinks
            .iter()
            .any(|c| c.id == parent && c.kind == crate::LinkKind::Transcludes));
        assert!(pv
            .backlinks
            .iter()
            .any(|c| c.id == sibling && c.kind == crate::LinkKind::References));
        assert!(pv.links_out.is_empty(), "child points at nothing");

        // The parent's outgoing edge resolves to the child.
        let pp = svc
            .page_view(&ctx, &parent, false, None, None)
            .unwrap()
            .unwrap();
        assert!(pp
            .links_out
            .iter()
            .any(|c| c.id == child && c.kind == crate::LinkKind::Transcludes));

        // Line range slices the raw body.
        let ranged = svc
            .page_view(&ctx, &child, false, Some(2), Some(2))
            .unwrap()
            .unwrap();
        assert_eq!(ranged.block.content, "line two\n");

        // rendered=true inlines the embedded child into the parent body.
        let rendered = svc
            .page_view(&ctx, &parent, true, None, None)
            .unwrap()
            .unwrap();
        assert!(rendered.block.content.contains("line one"));

        // A missing block is None, not an error.
        assert!(svc
            .page_view(&ctx, &BlockId::generate(), false, None, None)
            .unwrap()
            .is_none());
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

    // ----- human-only (locked) blocks -----

    /// The desktop-app context (full scope, incl. lock management). `RequestContext::local()` is
    /// the *agent* scope (read + write, no lock management).
    fn app() -> RequestContext {
        RequestContext::local_app()
    }

    #[test]
    fn set_lock_requires_manage_locks() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let agent = RequestContext::local();
        let id = svc.create_block(&agent, Some("Pinned"), "body").unwrap();
        // An agent (CLI/MCP) lacks ManageLocks → cannot lock or unlock.
        assert!(svc.set_lock(&agent, &id, true).is_err());
        // The app can.
        svc.set_lock(&app(), &id, true).unwrap();
        assert!(svc.get_block(&agent, &id).unwrap().unwrap().locked);
    }

    #[test]
    fn agent_can_read_but_not_write_locked_block() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let agent = RequestContext::local();
        let id = svc
            .create_block(&agent, Some("Pinned"), "the original")
            .unwrap();
        svc.set_lock(&app(), &id, true).unwrap();

        // Reads are allowed for an agent, and report the lock state.
        let rec = svc.get_block(&agent, &id).unwrap().unwrap();
        assert!(rec.locked);
        assert_eq!(rec.content, "the original");
        assert!(svc.get_block_source(&agent, &id).unwrap().is_some());
        assert!(svc.render_block(&agent, &id).unwrap().is_some());

        // Every write path is denied for an agent.
        assert!(svc
            .update_block(&agent, &id, None, "tampered", false, None)
            .is_err());
        assert!(svc.set_tags(&agent, &id, &["x".to_string()]).is_err());
        assert!(svc
            .set_props(&agent, &id, &[("k".to_string(), "v".to_string())])
            .is_err());
        assert!(svc.carve_block(&agent, &id, None, "chunk").is_err());
        assert!(svc.delete_block(&agent, &id).is_err());

        // The body is untouched after the denied writes.
        assert_eq!(
            svc.get_block_source(&agent, &id).unwrap().unwrap(),
            "the original"
        );
    }

    #[test]
    fn set_props_round_trips_and_survives_body_edit() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let id = svc.create_block(&ctx, Some("Atom"), "body v1").unwrap();

        // An agent can set arbitrary properties on an unlocked block.
        svc.set_props(
            &ctx,
            &id,
            &[
                ("source".to_string(), "https://example.com/x".to_string()),
                ("verified".to_string(), "2026-06-01".to_string()),
            ],
        )
        .unwrap();

        // get_block overlays the authoritative props from the vault.
        let rec = svc.get_block(&ctx, &id).unwrap().unwrap();
        assert_eq!(
            rec.props,
            vec![
                ("source".to_string(), "https://example.com/x".to_string()),
                ("verified".to_string(), "2026-06-01".to_string()),
            ]
        );

        // A plain body edit must not drop the properties.
        svc.update_block(&ctx, &id, Some("Atom"), "body v2", false, None)
            .unwrap();
        let rec = svc.get_block(&ctx, &id).unwrap().unwrap();
        assert_eq!(rec.content, "body v2");
        assert_eq!(
            rec.props,
            vec![
                ("source".to_string(), "https://example.com/x".to_string()),
                ("verified".to_string(), "2026-06-01".to_string()),
            ]
        );
    }

    #[test]
    fn search_hits_carry_overlaid_props() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let id = svc
            .create_block(&ctx, Some("Atom"), "findable body text")
            .unwrap();
        svc.set_props(&ctx, &id, &[("source".to_string(), "git".to_string())])
            .unwrap();
        let hits = svc.search(&ctx, SearchQuery::text("findable")).unwrap();
        let hit = hits
            .iter()
            .find(|h| h.block.id == id)
            .expect("block should be found");
        assert_eq!(hit.block.prop("source"), Some("git"));
    }

    #[test]
    fn get_block_carries_created_and_updated() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let id = svc.create_block(&ctx, Some("Doc"), "body").unwrap();
        let rec = svc.get_block(&ctx, &id).unwrap().unwrap();
        assert!(rec.created.is_some(), "created decoded from the id");
        assert!(rec.updated.is_some(), "updated stamped on create");
    }

    #[test]
    fn search_date_filters_select_recent_and_stale() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        svc.create_block(&ctx, Some("Doc"), "findable content here")
            .unwrap();

        let past = crate::clock::parse_query_date("2000-01-01");
        let future = crate::clock::parse_query_date("2999-01-01");

        // Just-written, so NOT updated before a past date, but IS before a far-future date.
        let mut q = SearchQuery::text("findable");
        q.updated_before = past.clone();
        assert!(svc.search(&ctx, q).unwrap().is_empty());

        let mut q = SearchQuery::text("findable");
        q.updated_before = future;
        assert_eq!(svc.search(&ctx, q).unwrap().len(), 1);

        // created is ~now, so created_after a past date matches.
        let mut q = SearchQuery::text("findable");
        q.created_after = past;
        assert_eq!(svc.search(&ctx, q).unwrap().len(), 1);
    }

    #[test]
    fn updated_before_excludes_block_without_updated() {
        // A block with no `updated:` time must not match an updated bound (unknown != in range).
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        let id = BlockId::generate();
        std::fs::create_dir_all(dir.path().join("blocks")).unwrap();
        std::fs::write(
            dir.path().join(format!("blocks/{id}.md")),
            "---\ntitle: Old\n---\n\nfindable old content\n",
        )
        .unwrap();
        svc.reconcile(&ctx).unwrap();
        let rec = svc.get_block(&ctx, &id).unwrap().unwrap();
        assert_eq!(rec.updated, None);
        assert!(rec.created.is_some());
        let mut q = SearchQuery::text("findable");
        q.updated_before = crate::clock::parse_query_date("2999-01-01");
        assert!(
            svc.search(&ctx, q).unwrap().is_empty(),
            "no-updated block must be excluded from updated_before"
        );
    }

    #[test]
    fn date_filter_scans_beyond_the_default_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        for _ in 0..60 {
            svc.create_block(&ctx, Some("Doc"), "findable content")
                .unwrap();
        }
        // Regression: a metadata filter must consider ALL blocks, not just the engine's default 50.
        // All 60 were just written, so all are updated before a far-future date. With a generous
        // limit, every one comes back — proving the candidate set wasn't pre-capped at 50.
        let mut q = SearchQuery::text("findable");
        q.updated_before = crate::clock::parse_query_date("2999-01-01");
        q.limit = 100;
        assert_eq!(svc.search(&ctx, q).unwrap().len(), 60);
    }

    #[test]
    fn has_and_missing_property_audit() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        // Two atoms with provenance, one without — the metadata-gap the audit must surface.
        let a = svc.create_block(&ctx, Some("A"), "alpha body").unwrap();
        let b = svc.create_block(&ctx, Some("B"), "beta body").unwrap();
        let c = svc.create_block(&ctx, Some("C"), "gamma body").unwrap();
        svc.set_props(&ctx, &a, &[("source".into(), "git".into())])
            .unwrap();
        svc.set_props(&ctx, &b, &[("source".into(), "doc".into())])
            .unwrap();

        // Pure presence query (no free text) — an audit over the whole vault.
        let q = SearchQuery {
            has_prop: vec!["source".into()],
            ..Default::default()
        };
        let got: Vec<_> = svc
            .search(&ctx, q)
            .unwrap()
            .into_iter()
            .map(|h| h.block.id)
            .collect();
        assert_eq!(got.len(), 2);
        assert!(got.contains(&a) && got.contains(&b) && !got.contains(&c));

        // Absence query — the part FTS cannot express.
        let q = SearchQuery {
            lacks_prop: vec!["source".into()],
            ..Default::default()
        };
        let got: Vec<_> = svc
            .search(&ctx, q)
            .unwrap()
            .into_iter()
            .map(|h| h.block.id)
            .collect();
        assert_eq!(got, vec![c.clone()], "only the atom missing source");

        // Compose: has source AND missing verified → only `a`.
        svc.set_props(&ctx, &b, &[("verified".into(), "2026-01-01".into())])
            .unwrap();
        let q = SearchQuery {
            has_prop: vec!["source".into()],
            lacks_prop: vec!["verified".into()],
            ..Default::default()
        };
        let got: Vec<_> = svc
            .search(&ctx, q)
            .unwrap()
            .into_iter()
            .map(|h| h.block.id)
            .collect();
        assert_eq!(got, vec![a]);
    }

    #[test]
    fn pure_missing_audit_scans_whole_vault_via_filter_only_path() {
        // The feature's headline path: a pure `missing:` query with NO free text routes through the
        // engine's filter-only (None, None) candidate path. Over a >50-block vault it must still
        // consider every block (not the default 50), so all gap-blocks come back with a high limit.
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let ctx = RequestContext::local();
        for _ in 0..60 {
            svc.create_block(&ctx, Some("Doc"), "body").unwrap();
        }
        let q = SearchQuery {
            lacks_prop: vec!["source".into()],
            limit: 100,
            ..Default::default()
        };
        assert_eq!(svc.search(&ctx, q).unwrap().len(), 60);
    }

    #[test]
    fn locked_block_is_immutable_even_to_the_app() {
        // Explicit-unlock model: a locked block has no write-through, not even for the app.
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let agent = RequestContext::local();
        let id = svc.create_block(&agent, Some("Pinned"), "v1").unwrap();
        svc.set_lock(&app(), &id, true).unwrap();
        // The app holds Write + ManageLocks, but a direct write to a locked block still fails.
        assert!(svc
            .update_block(&app(), &id, None, "v2", false, None)
            .is_err());
        assert_eq!(svc.get_block_source(&agent, &id).unwrap().unwrap(), "v1");
    }

    #[test]
    fn unlock_then_edit_then_relock() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let agent = RequestContext::local();
        let id = svc.create_block(&agent, Some("Pinned"), "v1").unwrap();
        svc.set_lock(&app(), &id, true).unwrap();
        assert!(svc
            .update_block(&agent, &id, None, "nope", false, None)
            .is_err());

        // The honest flow: app unlocks → edit → re-lock. After unlock an agent can write too.
        svc.set_lock(&app(), &id, false).unwrap();
        svc.update_block(&agent, &id, None, "v2", false, None)
            .unwrap();
        assert_eq!(svc.get_block_source(&agent, &id).unwrap().unwrap(), "v2");
        svc.set_lock(&app(), &id, true).unwrap();
        let rec = svc.get_block(&agent, &id).unwrap().unwrap();
        assert_eq!(rec.content, "v2");
        assert!(rec.locked, "re-lock must persist");
    }

    #[test]
    fn agent_cannot_carve_from_or_link_into_locked_block() {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = service(dir.path());
        let agent = RequestContext::local();
        let locked = svc.create_block(&agent, Some("Pinned"), "body").unwrap();
        let other = svc.create_block(&agent, Some("Other"), "x").unwrap();
        svc.set_lock(&app(), &locked, true).unwrap();

        // Carve-from and link-into both mutate the locked block → denied.
        assert!(svc.carve_selection(&agent, &locked, 0, 4).is_err());
        assert!(svc.link_blocks(&agent, &locked, &other, true).is_err());
        // But linking FROM an unlocked block TO the locked one is fine (target isn't mutated).
        assert!(svc.link_blocks(&agent, &other, &locked, false).is_ok());
    }
}
