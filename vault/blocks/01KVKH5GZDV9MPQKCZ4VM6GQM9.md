---
title: "Rule: Run the full suite before every commit"
tags: [doc, contributing]
---

### Always run the full suite before every commit — and it must be green

- Run **`cargo test --workspace`** before *every* commit. Zero failures, zero ignored-without-reason.
- Never commit with a red or skipped suite. If a test is legitimately pending, it is the
  task — don't commit around it.
