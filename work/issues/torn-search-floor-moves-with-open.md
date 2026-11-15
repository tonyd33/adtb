---
created-at: 2026-11-15
status: open
kind: bug
priority: 700
summary: "the torn-write search's per-kind case floor depends on which pages open writes"
tags:
  - test
  - class-weak-tests
---

# Torn search floor moves with open

## Report

`damage_search::a_torn_write_is_restored_from_its_image` (crates/adtb-sim/tests/storage/damage_search/mod.rs, the floor check near line 847) requires at least 8 cases per page kind. The cases come from the pages the workload's writes touch, so a change that only moves which pages open writes changes the count. In structure-lang-identity step 10b-2, moving the migrations read into open left Bitmap at 7 and failed the test ("Torn: kinds with fewer than 8 cases ... Bitmap: 7") with no change to torn-write recovery. The read was moved back.

The floor guards that the search reaches every page kind; it should hold by construction of the workload (a workload step that writes each kind enough times, or a floor computed from the kinds the workload is known to write), not by the incidental pages open writes.

Defect class: Tests asserting less than they claim: a coverage floor met by coincidence of unrelated code.
