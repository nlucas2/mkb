//! Footprint/latency/recall benchmark for the local ONNX embedder.
//!
//! Measures the *real* cost of the native `ort` runtime (not Python): query latency and
//! retrieval quality over a small, representative personal-KB corpus. Pair it with the OS to
//! capture peak resident memory, e.g.:
//!
//! ```sh
//! cargo build --release -p mkb-embed --features onnx --example footprint
//! /usr/bin/time -l ./target/release/examples/footprint   # macOS: "maximum resident set size"
//! /usr/bin/time -v ./target/release/examples/footprint   # Linux: "Maximum resident set size"
//! ```
//!
//! The corpus and queries mirror `files/retrieval_benchmark.py` so the Rust and Python numbers
//! are directly comparable. Requires the `onnx` feature.

use std::time::Instant;

use mkb_core::cosine_similarity;
use mkb_embed::{Embedder, FastEmbedder};

/// (id, lineage headings, content) — doc text embeds `contextual_text()` exactly like the engine.
const BLOCKS: &[(&str, &[&str], &str)] = &[
    (
        "b1",
        &["Homelab", "Web server"],
        "Bounce the nginx service: sudo systemctl restart nginx",
    ),
    (
        "b2",
        &["Homelab", "Database"],
        "Postgres won't come up — check /var/log/postgresql, usually a stale pid file",
    ),
    (
        "b3",
        &["Dev", "Networking"],
        "Free a stuck port: lsof -i :8080 then kill the PID",
    ),
    (
        "b4",
        &["Homelab", "TLS"],
        "Rotate the certificate with certbot renew --force-renewal",
    ),
    (
        "b5",
        &["Personal", "Coffee"],
        "Order: oat milk flat white, no sugar",
    ),
    (
        "b6",
        &["Homelab", "Raspberry Pi"],
        "Reboot the Pi remotely: ssh pi@host then sudo reboot",
    ),
    (
        "b7",
        &["Homelab", "Backups"],
        "Back up the vault nightly with restic to the NAS",
    ),
    (
        "b8",
        &["Homelab", "Kubernetes"],
        "Pod stuck in CrashLoopBackOff — kubectl describe pod, look for OOMKilled",
    ),
    (
        "b9",
        &["Dev", "JVM"],
        "Bump the heap: set -Xmx4g in the startup flags",
    ),
    (
        "b10",
        &["Personal", "Garage"],
        "Garage door remote battery is a CR2032",
    ),
    (
        "b11",
        &["Dev", "macOS"],
        "Flush DNS cache: sudo dscacheutil -flushcache",
    ),
    (
        "b12",
        &["Personal", "Home"],
        "Wifi password is taped under the router",
    ),
    (
        "b13",
        &["Personal", "Domains"],
        "Renew the domain registration before it lapses in March",
    ),
    (
        "b14",
        &["Homelab", "Kubernetes"],
        "Disable swap before installing kubeadm: swapoff -a",
    ),
    (
        "b15",
        &["Dev", "Disk"],
        "Find what's eating disk: du -ah / | sort -rh | head",
    ),
];

/// (query, gold block id) — worded differently from the source, the point of semantic search.
const QUERIES: &[(&str, &str)] = &[
    ("restart the web server", "b1"),
    ("make the website serving process come back up", "b1"),
    ("database is down", "b2"),
    ("the container keeps dying and restarting", "b8"),
    ("clear the dns cache", "b11"),
    ("flush dns cache", "b11"),
    ("disk is full how do I find big files", "b15"),
];

fn contextual_text(lineage: &[&str], content: &str) -> String {
    if lineage.is_empty() {
        content.to_string()
    } else {
        format!("{}\n\n{}", lineage.join(" > "), content)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load vendored local files (no network) — the only supported path. Point MKB_MODEL_DIR
    // at a model directory (e.g. the int8 BGE-small export) to benchmark it.
    let dir = std::env::var("MKB_MODEL_DIR").map_err(|_| {
        "set MKB_MODEL_DIR to a model directory (ONNX + tokenizer files) to run this benchmark"
    })?;
    let load_start = Instant::now();
    println!("loading from MKB_MODEL_DIR={dir}");
    let embedder = FastEmbedder::from_model_dir(&dir, 384, "bge-small")?;
    let load_ms = load_start.elapsed().as_millis();
    println!("model: {} (dim {})", embedder.model_id(), embedder.dim());
    println!("load:  {load_ms} ms");

    let doc_texts: Vec<String> = BLOCKS
        .iter()
        .map(|(_, l, c)| contextual_text(l, c))
        .collect();
    let doc_vecs = embedder.embed(&doc_texts)?;

    let mut latencies_ms: Vec<u128> = Vec::with_capacity(QUERIES.len());
    let mut r1 = 0usize;
    let mut r3 = 0usize;
    for (query, gold) in QUERIES {
        let t = Instant::now();
        let qv = embedder.embed_one(query)?;
        latencies_ms.push(t.elapsed().as_millis());

        let mut scored: Vec<(&str, f32)> = BLOCKS
            .iter()
            .zip(&doc_vecs)
            .map(|((id, _, _), dv)| (*id, cosine_similarity(&qv, dv)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let top3: Vec<&str> = scored.iter().take(3).map(|(id, _)| *id).collect();
        if top3.first() == Some(gold) {
            r1 += 1;
        }
        if top3.contains(gold) {
            r3 += 1;
        }
        println!("  q={query:?} gold={gold} -> {top3:?}");
    }

    let n = QUERIES.len();
    latencies_ms.sort_unstable();
    let median = latencies_ms[n / 2];
    let mean = latencies_ms.iter().sum::<u128>() as f64 / n as f64;
    println!("query latency: median {median} ms, mean {mean:.1} ms over {n} queries");
    println!(
        "recall@1 = {:.3}   recall@3 = {:.3}",
        r1 as f64 / n as f64,
        r3 as f64 / n as f64
    );
    Ok(())
}
