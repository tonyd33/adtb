---
created-at: 2026-11-08
status: open
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
