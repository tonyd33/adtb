---
created-at: 2026-10-27
status: resolved
closed: 2026-11-15
kind: bug
priority: 590
summary: "bundle and registered rows decode an unknown Registered constructor as ImportedFn, a malformed hides_new as false, and Option fields by constructor position"
tags:
  - quality
  - engine
  - class-identity-by-spelling
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

## Resolved

Resolved by structure-lang-identity step 10b-1, yzxywwvs c3d9181c (2 rounds, full check 0, 3820 tests). `catalog::bundles` and `catalog::registered` rows decode through `catalog::codec` codecs: constructors and nested sums (`BundleState`, `Priority`, `Registered`) by engine fixed id, nested payload types checked against catalog.adt when the codec is built, Option and Bool on the prelude constants. A malformed row refuses at open as damage with its whole reason; no default remains (`hides_new`, `version`, the `ImportedFn` fallback are gone); `CallRow::kind` is the `Callable` enum. A stored bundle name must read as a bundle name and match its family, each with its own refusal. Dump's bundle, registered, feed and feed_member readers use the same codecs (restore keeps `damaged(SECTION)`). Reversed-catalog sample tests cover every nested constructor.
