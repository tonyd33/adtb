---
created-at: 2026-11-08
status: resolved
closed: 2026-11-16
kind: feature
priority: 17
summary: "the catalog's roots heap is 16 hand-numbered constructors decoded at 5 sites, and naming_sequences is stored state no rule owns"
---

# Feature: Catalog roots rows owned by one codec

## Request

Part of the structural program. Moved out of structure-version-owner by the lead on 2026-11-08 (its Q6): the roots heap and `naming_sequences` are catalog stored state, owned by no version rule.

## Proposal

One `catalog::roots::RootsRow` enum owns the roots heap's constructors and their ids, with one encoder and decoder. Exit: roots constants outside roots.rs 16 to 0; `[Value::Sum { ctor, fields }]` roots decoders outside it 5 to 0; a round trip over every variant; the catalog crash sweeps green. `naming_sequences` gets a stated owner in the same module.

Source: work/briefs/design-version-owner-r1-report.md step 11 and work/briefs/audit-structure-r1-data.md (catalog: catalog/mod.rs:196-236 with heap/row.rs:239-244, roots constructor position equals id only by list order). Related: structure-lang-identity (catalog ids by position and name).

## Open questions

## 2026-11-16 status

The quality review of 2026-11-16 found most of the exit met by structure-lang-identity step 11e (f6a76796): one `RootsRow` in catalog/roots.rs with its encoder and decoder, a round trip over every variant. Left: `naming_sequences` has no stated owner in the module. The ticket stays open for that alone.

## 2026-11-16, resolved

Landed as uytwqlrl cadb3d57 (2 rounds), full check 0 (3858 tests). The root page's `changing` marker, which records the catalog change in progress (a schema change, a reindex, the naming of the counters page), is private to catalog/root.rs: a change begins through `begin` or `begin_or_join` and ends only through the `Begun` token it got, or through the open's `end_found`. Assignments of `changing` outside root.rs: 11 to 0, by the type. Fixed on the way: `counters` cleared a naming marker an open had kept after a crash, so the crashed run's page was never freed (class-not-atomic; regression test in catalog_tests.rs). Follow-up: concurrent-rebuilds-share-marker.
