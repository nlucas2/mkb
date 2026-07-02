---
title: "Rule: Keep the seams clean"
tags: [doc/contributing]
updated: 2026-07-02T06:45:42Z
---

### Keep the seams clean

- Pluggable boundaries are traits (`Index`, `Embedder`, `IdCodec`, transport). Program to the
  trait, not the concrete type, so engines/encodings can be swapped without touching callers.
- Don't leak storage/encoding details past their trait boundary.
