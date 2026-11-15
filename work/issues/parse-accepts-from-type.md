---
created-at: 2026-11-14
status: open
kind: feature
priority: 180
summary: "a pinned parse decides its accepted constructors from a narrowed static type"
tags:
  - lang
---

# Feature: carry a pinned parse's accepted constructors in the operation

## Report

Left by the translate track (ookywzux 9751992c). Translation types a pinned call's `Parse` result with `Map::ty_parsed` (`Narrow::All`, translate/map.rs), narrowing value sums to the pin's constructors, and the engine builds the parse form from the result type (exec/prim/mod.rs), so the narrowed static type decides which constructors a parse accepts. That is the shape the ruling removed for `Deref`: a run-time restriction carried as a static type the engine reads back. Value sums elsewhere keep their declared type. Carry the accepted constructors in the parse operation (as `Deref` carries `admits`) and type the result as declared. No result moves today: both pinned print/parse bundles tests pass.

Defect class: none (one instance).
