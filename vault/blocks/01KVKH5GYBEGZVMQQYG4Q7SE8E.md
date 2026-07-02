---
title: "Rule: Tests are mandatory"
tags: [doc/contributing]
updated: 2026-07-02T06:45:41Z
---

### Tests are mandatory

- Every behavior change ships with tests in the **same change**. No "I'll add tests later."
- New modules with logic are not "done" until they have unit tests covering the happy path
  **and** the meaningful edge cases.
- Bug fixes start by adding a test that reproduces the bug, then fixing it.
