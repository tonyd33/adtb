---
created-at: 2026-11-15
status: open
kind: bug
priority: 850
summary: "kernel.md section 3 says a Record or Ctor evaluates its fields in name order, and the next paragraph says in no stated order"
tags:
  - quality
  - design
  - class-none
---

# kernel.md states two evaluation orders for a record's fields

## Report

Found by the quality review of reviewed..582d7af5. docs/design/kernel.md, after the §3 table: line 295 says "A `Record` or `Ctor` evaluates its fields in the order the node lists them, which is name order." Line 298 says "Fields of a record or constructor are evaluated in no stated order: which of several faulting fields is reported is a freedom of semantics.md §5.1, so a consumer may evaluate them in name order." A consumer cannot tell whether name order is a promise or a permission, and so whether a reported fault may differ between the engine and the reference.

Fix direction: keep the second (the spec's freedom) and delete the first, or state name order as the kernel's contract and drop the freedom; one statement.
