---
title: "Rule: Do not duplicate code"
tags: [doc, contributing]
---

### Do not duplicate code — always look for reuse

- Before writing new logic, search for an existing implementation to call or extend.
- If the same logic is needed in two places, **extract it into `mkb-core`** (or a shared
  module) and call it from both. Copy-paste of logic is a defect.
- Prefer extending a shared function over forking a near-duplicate.
