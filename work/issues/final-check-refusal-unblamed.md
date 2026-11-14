---
created-at: 2026-11-14
status: open
kind: bug
priority: 220
summary: "a migration refused by its schema check after its operators reports an empty operator"
tags:
  - lang
  - class-none
---

# a migration refused by its schema check after its operators reports an empty operator

## Report

its schema check after its operators reports an empty operator". `api/migrate.rs` maps
`catalog::Error::Refused` to `SchemaRefused { operator: "" }`. Repro:
`crates/adtb-sim/scripts/oplog/remove-constructor-leaving-index-over-every-constructor-refused.test`;
its expected `operator: ""` moves to the operator's text with the fix.


Defect class: none (the refusal's blame is lost; no other instance known).
