---
created-at: 2026-10-26
status: open
kind: feature
priority: 791
summary: "the spec leaves open the row type of explain analyze, and the ids and seeding of catalog::settings rows"
tags:
  - spec
---

# Feature: Command gaps in the spec

## Report

Found by the engine commands track:
- catalog.md §7.2 shows `explain analyze` returning `{ text, document, change: {...}, constraints: checked }`, but no row type is stated. The engine returns `{ document, text }` and puts `change` and `constraints` in both.
- The ids of `catalog::settings` rows, and whether a new file holds one row per setting constructor, are undecided. The engine uses the setting constructor's catalog id. Example: `set setting RetryLimit = { retries: 3 }` then `from s in catalog::settings select key s`.
- With no statistics, `estimate` is `null`; the spec does not say whether that is allowed.
