---
created-at: 2026-11-12
status: open
kind: bug
priority: 450
summary: "a rename, and a revert respelling a name, can write a name a local binder captures, silently changing what the text reads"
tags:
  - class-identity-by-spelling
---

# A renamed name is captured by a local binder

## Report

`add check zzw on ts (match n | o9 -> o9 != o)`, then `rename field T.o to o9`: the check becomes `match n | o9 -> o9 != o9`, comparing `n` with itself. evolution.md §4.5: "A rename changes a name and nothing else". The same through a revert: drop the check, rename the field, revert the drop, and the restored text reads `o9 != o9`. And for relations: `define fn zzr (us9: Set (U)) -> ... = from u in us select ...`, dropped, `rename relation us to us9`, revert: `from u in us9` now reads the parameter. The law search sees no difference because its target is built with the same rename.

Cause: a bare field in a check body is a scope-local noted as a field (check/expr.rs:407), and locals shadow unqualified relations and functions (check/expr.rs:402, check/call.rs:150). The forward rename rewrite and `Spelled::spelled` (evolve/revert/spelled.rs) write the new spelling with no capture check. Found in the delta review of structure-lang-identity step 5c-2; forward rename pre-existing.

Direction: one gate for both paths: after respelling, re-check the text and refuse (or qualify) when a name piece no longer resolves to the ids it carries. Test: both probes in the law search, compared by resolved ids, not text.

Defect class: Catalog rows and identity by position or spelling.
