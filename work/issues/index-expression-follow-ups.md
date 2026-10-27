---
created-at: 2026-10-26
status: open
kind: bug
priority: 746
summary: "an index expression that faults has no stated behaviour, redefining a function an index calls leaves stale keys, and derived elements do not count for admission"
---

# Index expression follow ups

## Report

## Report

From the expression index track:
- No spec says what happens when an index expression faults on a row (`index (100 / n)` with `n: 0`). The engine fails the write with the fault, or refuses the migration. State it in `docs/spec/constraints.md`.
- `rewritten` in `crates/adtb/src/catalog/change.rs` compares element text, so redefining a user function an expression index calls leaves stale keys. Rebuild on the function's code hash, and state when in the spec.
- `expr_text` in `crates/adtb/src/catalog/rows.rs` falls back to Debug output for `match`, records and comprehensions, so their catalog clause is not as written.
- `crates/adtb/src/write/delta` uses only field indexes for index-bounded admission; path and expression elements do not count.
- At open and migrate the source is checked and normalised several times (`admits`, `build`, `validate`, the API).
