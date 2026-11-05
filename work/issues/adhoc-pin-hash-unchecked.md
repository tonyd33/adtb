---
created-at: 2026-11-04
status: open
kind: bug
priority: 420
summary: "An ad hoc at block's pins N (HASH) reads with any hash; the spec does not say whether it is checked"
---

# Ad hoc at block pin hash is not checked

## Report

## Report

`at commit P pins 1 (H) { ... }` sent ad hoc reads normally for any 64 hex digits `H`. A bundle's header hash is checked (applications.md, bundle pins); derived.md §6.3 does not say whether an ad hoc block's hash is. No test can tell: a harness mutant hashing the wrong prefix of the log failed only the bundle header script.

Found in the review of script batch 5, 2026-11-04 (reviewer's scratch script, not kept).

## Proposal

Rule in the spec whether an ad hoc block's pin hash is checked against the log's first N migrations. If it is, refuse a mismatch with the same refusal a bundle gets and add a script for it.
