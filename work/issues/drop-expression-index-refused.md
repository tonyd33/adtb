---
created-at: 2026-11-15
status: open
kind: bug
priority: 320
summary: drop index of an expression index is refused as an object in neither the source before nor after
tags:
  - lang
  - class-identity-by-spelling
---

# Drop expression index refused

## Report

Found by structure-lang-identity step 10c-2 on cf92f157. `drop index (lower(name)) in items` is refused with "an object ... is in neither the source before it nor the one after it", from crates/adtb-lang/src/evolve/resolved.rs:735. Nothing in the repository drops an expression index, so the path is untested. A field-path index drops.

Expected: dropping a declared expression index applies, matching the declaration by the element it denotes, not by its spelling.

Cause not yet established; the class tag is provisional (the lookup that pairs the operator with the declaration is the first suspect).

Defect class: Catalog rows and identity by position or spelling (provisional).
