---
created-at: 2026-11-13
status: open
kind: bug
priority: 850
summary: "Source::built gives a synthesized module header FileId(u32::MAX - 2), a third reserved file id with no named constant beside PRELUDE and ENGINE"
tags:
  - lang
  - quality
  - class-none
---

# A synthesized module header uses an unnamed reserved file id

## Report

crates/adtb-lang/src/evolve/source.rs:132 pushes an empty module header for a module that has no declarations as `FileId(u32::MAX - 2)`. crates/adtb-lang/src/span.rs:86 names the other reserved file ids, `FileId::PRELUDE` and `FileId::ENGINE`. This one has no name, so nothing ties it to them, and a new reserved id could take the same value. Step 6b's sentinel count does not include it.

Found by reading.

Fix direction: a named `FileId` constant beside `PRELUDE` and `ENGINE`, used at this site.

Defect class: class-none
