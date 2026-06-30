# Vendored UI assets (offline)

mkb ships these third-party files so the desktop app has **zero network/CDN
dependencies** at runtime. Do not hand-edit the `.min.js` files; re-vendor from
the upstream version noted below.

| File(s) | Upstream | Version | License |
|---|---|---|---|
| `highlight.min.js`, `highlight-theme.css` | [highlight.js](https://github.com/highlightjs/highlight.js) (common bundle + atom-one-dark) | 11.11.1 | BSD-3-Clause |
| `hljs-dockerfile.min.js`, `hljs-powershell.min.js`, `hljs-nginx.min.js`, `hljs-properties.min.js` | highlight.js language modules (not in the common bundle) | 11.11.1 | BSD-3-Clause |
| `hljs-kql.min.js` | [highlightjs-kql](https://github.com/siliconcupcake/highlightjs-kql) — Kusto/KQL grammar | compiled for hljs 11.11.x | MIT |
| `force-graph.min.js` | [force-graph](https://github.com/vasturiano/force-graph) | vendored | MIT |
| `mermaid.min.js` | [mermaid](https://github.com/mermaid-js/mermaid) — diagram rendering for ` ```mermaid ` fences | 11.16.0 | MIT |

## Syntax highlighting coverage

The highlight.js **common** bundle already covers the heavy hitters: C/C++, C#,
Go, Rust, Python, JavaScript/TypeScript, Java, Kotlin, Swift, Ruby, PHP, SQL,
shell/bash, JSON, YAML, TOML/INI, XML/HTML, Markdown, diff, and more.

We add, on top of the common bundle:

- **Dockerfile**, **PowerShell**, **nginx**, **.properties** — common ops/config
  languages missing from the common bundle.
- **Kusto / KQL** (`kusto`, `kql`, and aliases `azuremonitor` / `loganalytics`)
  via the community `highlightjs-kql` grammar. We register the `kusto` alias in
  `index.html` since fenced blocks are commonly tagged ` ```kusto `.

Unknown languages degrade gracefully to plain (uncolored) text.

### Re-vendoring

```sh
V=11.11.1
BASE="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/$V"
curl -fsSL "$BASE/highlight.min.js" -o highlight.min.js
curl -fsSL "$BASE/styles/atom-one-dark.min.css" -o highlight-theme.css
for L in dockerfile powershell nginx properties; do
  curl -fsSL "$BASE/languages/$L.min.js" -o "hljs-$L.min.js"
done
curl -fsSL "https://cdn.jsdelivr.net/gh/siliconcupcake/highlightjs-kql/dist/kql.min.js" -o hljs-kql.min.js
curl -fsSL "https://cdn.jsdelivr.net/npm/mermaid@11.16.0/dist/mermaid.min.js" -o mermaid.min.js
```

## Mermaid diagrams

` ```mermaid ` fenced blocks render as diagrams in the desktop app (offline; the
vendored build exposes `globalThis.mermaid`). Each diagram has a **Source** toggle
to flip to the raw text and back, and a parse/render failure falls back to the
source with an error note. Mermaid is browser-only, so this is a desktop-app
surface: the `mkb-view` crate (exports / any web surface) still emits the raw
fence, which GitHub and most Markdown viewers render themselves.
