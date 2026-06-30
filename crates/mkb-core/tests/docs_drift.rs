//! Docs-as-data drift gate, in-process.
//!
//! Generated docs (README.md, AGENTS.md, the skills, …) are produced from source blocks in
//! `vault/` via the manifest in `vault/export.toml`. This test re-runs that export against the
//! repo's own vault and asserts every generated file on disk still matches — exactly what
//! `mkb export vault --check` did as a CI shell step, but now part of `cargo test --workspace`,
//! with no daemon, no binary, and no shelling out (export is a pure function of the vault).
//!
//! If this fails: you edited a source block (or hand-edited a generated file) without
//! regenerating. Run `mkb export vault` and commit the regenerated docs.

use std::path::{Path, PathBuf};

use mkb_core::export::{merge_path_prop_entries, plan_exports, Manifest};
use mkb_core::{read_block_files, Vault};

/// The workspace root, derived from this crate's compile-time manifest dir (`crates/mkb-core`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repo root")
}

/// Load the repo's vault (all blocks) into an in-memory [`Vault`] — no index needed, since export
/// resolves transclusions directly from the vault.
fn load_vault(vault_dir: &Path) -> Vault {
    let mut vault = Vault::new();
    for (id, _path, source) in read_block_files(vault_dir).expect("read vault blocks") {
        vault.insert_source(id, &source);
    }
    vault
}

#[test]
fn generated_docs_match_their_source_blocks() {
    let root = repo_root();
    let vault_dir = root.join("vault");
    let manifest_text =
        std::fs::read_to_string(vault_dir.join("export.toml")).expect("read vault/export.toml");
    let mut manifest = Manifest::parse(&manifest_text).expect("parse export.toml");

    let vault = load_vault(&vault_dir);
    // The default export.toml flow also derives docs from blocks' `path`/`filename` properties
    // (mkb export --from-props), so the gate must too — otherwise prop-routed docs (the skills)
    // would look unmapped.
    merge_path_prop_entries(&vault, &mut manifest);
    let planned = plan_exports(&vault, &manifest).expect("plan exports");
    assert!(!planned.is_empty(), "manifest produced no docs");

    let mut drifted = Vec::new();
    for doc in &planned {
        // Manifest paths are relative to the repo root (e.g. "README.md", "docs/SPEC.md").
        let on_disk = root.join(&doc.path);
        let current = std::fs::read_to_string(&on_disk).ok();
        if current.as_deref() != Some(doc.content.as_str()) {
            drifted.push(doc.path.clone());
        }
    }

    assert!(
        drifted.is_empty(),
        "generated docs are out of date with their source blocks: {drifted:?}\n\
         edit the block in vault/, then run `mkb export vault` and commit the result.",
    );
}
