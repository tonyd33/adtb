---
created-at: 2026-10-27
status: open
kind: bug
priority: 895
summary: "bundle and registered rows decode an unknown Registered constructor as ImportedFn, a malformed hides_new as false, and Option fields by constructor position"
tags:
  - quality
  - engine
---

# Bundle rows decode malformed values as defaults instead of refusing them

## Report

The catalog rows for bundles and registered calls are read leniently in `crates/adtb/src/api/bundle.rs`:

- `call_row` decodes any constructor other than `Query` or `Txn` as `ImportedFn` (`bundle.rs:379`);
- `bundle_row` reads `hides_new` with `truth().unwrap_or(false)` (`bundle.rs:328`), where every other field refuses a malformed value;
- the `some` closure (`bundle.rs:291`) decodes Option by constructor position;
- `CallRow::kind` and `Header::priority` are name strings.

A corrupt or future-shaped row therefore reads as a different, valid one.

Fix: refuse those values as corrupt, make `kind` and `priority` enums, and decode Option by constructor identity, not position (the same class as `bool-ctor-by-position`).
