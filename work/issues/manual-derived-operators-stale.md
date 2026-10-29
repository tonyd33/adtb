---
created-at: 2026-10-27
status: in-progress
kind: bug
priority: 898
summary: "the manual says add default and add computed are refused although both now apply, and gives the old refusal for store and script"
branch: manual-derived
tags:
  - quality
  - devex
---

# The manual marks working default and computed-field migrations unavailable

## Report

At `c317ef9a`, `docs/manual/README.md:83-87` says the engine refuses `script`, `store`, `add default` and `add computed` with "this operator has no resolved form yet". `docs/manual/reference/language.md:611-615` repeats that refusal for defaults, computed fields and scripts.

Commit `6be9c898` added resolved forms and enabled engine application of defaults and computed fields before the manual commit landed. `crates/adtb-lang/src/evolve/derived.rs:13-77` resolves them, and `crates/adtb-sim/tests/oplog.rs:1398-1501` exercises their application, reads, drops and reverts. `store` is now accepted too, though physical storage and maintenance remain incomplete as [stored-views.md](stored-views.md) records. A script now fails with "the engine does not run a `script` yet" from `crates/adtb/src/catalog/mod.rs:513-520`, not the old resolved-form refusal.

Verification command from the review workspace:

```
direnv exec /home/tony/code/adtb work/tools/suite -p adtb-sim --test oplog -E 'test(an_added_default) | test(an_added_computed) | test(a_stored_view)'
```

Actual: five tests passed, including added-default application/revert, computed-field reads/catalog rows, and accepted stored-view reads. Expected: the manual distinguishes those working operations from physical stored-view maintenance and from refused scripts, and names current refusals accurately.

Fix those availability lists and add runnable migration examples for defaults and computed fields. The prose is outside the fenced examples, so the manual's passing executable examples do not catch this contradiction. [user-manual.md](user-manual.md)'s latest open work is reader sign-off, not this specific availability regression.

## 2026-10-28: parallel manual correction track

Fresh manual-derived workspacef8c8ab07 on999926a8 owns the manual README,
reference/language and evolving-a-schema guide as needed. Verify actual source and
correct only default/computed/store/script availability and associated runnable
examples. No normative spec, generated navigation/harness, dependency or hosting
change. Other stale claims are reported separately. Native tooling owns checks.

Edit/native mdBook checks may run alongside permissions and campaign work. Rust
executable manual page and focused oplog checks take the second serialized build
slot after campaign author releases the first; no full root branch suite for this
prose-only scope. Fresh reading review follows passing executable examples. Lead
owns scoped main docs/manual checks and combined full main after app/permissions
landings. Landing manual first, campaign second, permissions last. Record exact
source/config/commands/exits/logs/timing and all failure/fixture distinctions in
local work/briefs/*manual-derived* notes; essential scope remains this tracked ticket.

## 2026-10-28: native example harness and frozen prose

Author confirmed existing authored console examples execute through adtb-shell's
manual integration test, not adtb-manual's Rust API/errors page tests. Use the
existing shell every_page_prints_what_it_shows test; it runs all pages with the
actual engine only, no reference evaluator claim. No new harness is needed.
Second build slot was explicitly released after campaign checks completed.

Candidate44b743c9 on999926a8 changes only the three assigned manual pages. New
examples distinguish existing fill2, later default3/explicit4, and computed total
after update. Native docs check already collected0 (ten Python tests,422syntax
inputs,61pages,6736links,5001fragments,zero failures) on stable inputs; executable
shell/oplog gates and fresh reading review remain required before acceptance.
