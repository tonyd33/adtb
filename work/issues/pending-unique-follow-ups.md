---
created-at: 2026-10-28
status: open
kind: feature
priority: 860
summary: "a held old key's index ids come from a reserved range above 1<<31 kept apart from log ids by convention, and the review of held uniques left latent gaps and missing tests"
tags:
  - engine
---

# Feature: pending unique follow-ups

## Report

From the independent review of pending-unique-drops (report work/briefs/review-pending-unique.md, local).

- `crates/adtb/src/catalog/pending.rs` (about lines 35 and 63): a held old key's index ids are allocated upward from `1<<31`, written to disk and kept apart from log index ids only by convention; they restart at `1<<31` once nothing is held, so ids are reused. Give a relaxing rekey's new key a fresh log index id in adtb-lang, so the old key keeps its own id and physical index. Until then, fail `Internal` on a log index id at or above `1<<31`. Related to schema-id-sentinels.
- `crates/adtb/src/write/apply.rs` (about line 293): a unique violation on an index with no name is dropped and the write goes through. Latent: every unique index now has a name. Expect a name for a unique index, as the key path does.
- `crates/adtb/src/write/schema.rs` (about line 216): `rels` treats any undeclared unique index over plain fields as the old key, by its shape. Pass `rels` the catalog's index specs instead.
- `crates/adtb/src/catalog/readers.rs` (about lines 238 and 732): a pending drop's display name is text fixed at drop time and goes stale after a field rename; the tests at `crates/adtb-sim/tests/write_states.rs` 1284 and 1315 assert the stale text.
- readers.rs (about line 722): an old key is held by every live bundle whose version has the relation, not only those whose version has that key, so after two relaxing rekeys the second old key takes effect later than `evolution.md` section 6.4 says.
- readers.rs (about line 711): a relation whose old key fields lack catalog ids is skipped silently; it should fail `Internal`.
- `crates/adtb/src/catalog/rows.rs` (about line 140): a held old key's index spec is labelled `kind: "PrimaryKey"`.
- Tests: `crates/adtb-sim/tests/bundles.rs` 1118 checks the violated constraint's name by substring; `stays_pending` never asserts the schema has lost the unique; no crash test for a migration that adds a hold; none for a held unique over an expression, a scoped unique, or two old keys held at once; the carry-and-refuse tests have no unheld pair.
- The two new crash sweeps take 2.2 s and 2.4 s with no small sampled default.
