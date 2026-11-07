---
created-at: 2026-11-07
status: open
kind: bug
priority: 145
summary: "under the deep profile an_unreadable_page_fails_open_unmarked fails at seed 20: an open with latent read faults leaves a Database mark or changes the marks page bytes"
tags:
  - class-failure-flattened
---

# An open that meets an unreadable page writes the marks page at seed 20

## Report

## 2026-11-07

Found while checking the damage search: on main, `--profile deep` fails `open_damage::an_unreadable_page_fails_open_unmarked` (crates/adtb-sim/tests/storage/open_damage.rs:984) with "seed 20": either an open under latent read faults holds a `Database` mark or the marks page bytes change. storage.md §8: an open that meets an unreadable page refuses it unmarked and writes nothing. Passes at the default profile. Read which assertion fails first.

Defect class: Damage or failure gives a panic, hang or flattened error.
