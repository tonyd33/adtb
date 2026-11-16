---
created-at: 2026-11-16
status: open
kind: bug
priority: 600
summary: "a late key selection is held as its written text, so the same set written in another order is refused where it is expected"
tags:
  - lang
  - class-identity-by-spelling
---

# Late selection held as text

## Report

`KeyCtors::Late(String)` (crates/adtb-lang/src/types.rs) holds a selection left for the migration's end as its written text. `admits_late_key` (check/ty.rs, committed-schema-type part 2b-ii) starts with `a == b` over `Ty`, so in a script or fill a late key given as the same late selection written in another order (`Key regions.(Town | City)` as `Key regions.(City | Town)`) is refused "a script may not give ...", where the same spelling is accepted. An over-refusal only: one spelling on one relation names one selection.

Fix direction: hold the late selection by identity (the relation and the named set or the sorted constructor names as written members resolved at the end), keep the text for display only; a test giving one selection written two ways.
