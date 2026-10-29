---
created-at: 2026-10-29
status: in-progress
kind: bug
priority: 250
summary: "a hide new constructors bundle follows a reference into a constructor its pin lacks, so a deref returns a value or panics instead of faulting NoSuchRow"
branch: hide-new-deref
tags:
  - quality
  - engine
  - lang
---

# A hide new constructors bundle sees rows its pin lacks through a reference

## Report

`docs/spec/semantics.md` (about line 1445) and `docs/spec/applications.md:184` say a bundle declaring `hide new constructors` keeps only its pinned version's constructors in references too, so a `deref` of a row of a newer constructor faults `NoSuchRow`.

`Map::ty` (`crates/adtb-lang/src/translate/map.rs:179` and `:185`) translates every `Ty::Data` and `Ty::Key` constructor set with `self.set(..., false)`; only `Kind::Rel` passes `hide_new` (`crates/adtb-lang/src/translate/walk.rs:228`). `Walk::ty` (`walk.rs:90`) uses `Map::ty` for every term and variable, so a pinned `Key identities` with no constructor set covers every current constructor, and `lookup` (`crates/adtb/src/plan/lower/probe.rs:24-44`) probes the new constructor's heap too.

Effects:
- a field read through the reference (`s.who.user`) returns a value where the spec says `NoSuchRow`;
- returning the row whole panics in `pinned::back` (`crates/adtb/src/api/pinned.rs:41`);
- a `match` on the row reaches the `Kind::Unreachable` arm built at `walk.rs:864-867`.

Repro sketch (from the code, not run): in `crates/adtb-sim/tests/bundles.rs`, open `[V1, ADD_APPLE, ENABLE_APPLE]`, register a bundle pinned at V1 with ` hide new constructors` and `query q () -> Set Text = from s in sessions select s.who.user`, insert a session whose `who` is an Apple identity, call `q`. Expected `NoSuchRow`.

`hide_new_constructors_exempts_whole_reads_and_references` (`bundles.rs:637`) only registers its queries and never calls them.

Fix: under `hide_new`, translate `Ty::Key` and `Ty::Data` constructor sets to the pinned constructors, as `Map::set` does for `Kind::Rel`. Add call tests for a deref, a whole-row return and a match through a reference to a hidden row.

## 2026-10-29: dispatched

A fresh writer fixes this in workspace hide-new-deref, call tests first. Local brief: work/briefs/brief-hide-new-deref.md.
