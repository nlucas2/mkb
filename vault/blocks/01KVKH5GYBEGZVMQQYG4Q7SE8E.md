---
title: "Rule: Tests are mandatory"
tags: [doc, contributing]
---

### Tests are mandatory

- Every behavior change ships with tests in the **same change**. No "I'll add tests later."
- New modules with logic are not "done" until they have unit tests covering the happy path
  **and** the meaningful edge cases.
- Bug fixes start by adding a test that reproduces the bug, then fixing it.
