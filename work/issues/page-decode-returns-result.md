---
created-at: 2026-11-16
status: open
kind: feature
priority: 115
summary: "decoding a row or key from a page panics on a stored tag or form it does not know, where damage should be reported"
tags:
  - engine
  - class-failure-flattened
---

# Feature: Page decode returns result

## Request

Mechanism for `class-failure-flattened`, from the cause review of 2026-11-16. Reading a stored row or key panics when the page holds a tag or form the reader does not know, so damage on a page becomes a process abort instead of a reported finding:

- crates/adtb/src/heap/row.rs:239-244: `ctor_index` panics "stored tag {id} names no constructor" on a tag read from the page (instance: heap-unknown-stored-tag-panics).
- crates/adtb/src/catalog/layout.rs:127: `t => panic!("{t:?} is not a sum")`.
- the `unreachable` arms of catalog/codec.rs readers that a damaged row can reach.

Rulings 10b-1 and 10b-2 of structure-lang-identity made an arm "a file cannot reach" an invariant panic and a codec refusal at statement time internal "since the heap already decoded it"; the heap decode itself is not damage-safe, so that premise fails under damage.

## Proposal

Every decode of bytes read from a page returns `Result` with a typed damage cause (`Rot`), carried to verify as a finding and to a statement as an internal error naming the page. A structure-check row counts `panic!`, `unreachable!` and `unwrap_or_else(|| panic!` in crates/adtb/src/heap, catalog/codec.rs and catalog/layout.rs and only falls. A searching test corrupts the tag and form bytes of one row of each kind and asserts the finding whole.

Exit: the row at 0 for decode paths (invariants on values the engine built in memory may stay, each named); heap-unknown-stored-tag-panics resolved.

## Open questions

- Which of the counted sites decode page bytes and which only values built in memory: count first.
