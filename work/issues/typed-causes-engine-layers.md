---
created-at: 2026-11-03
status: open
kind: bug
priority: 340
summary: "catalog, write, plan and exec still pass Internal and Corrupt causes as text through Broken::Write, Broken::Catalog and Broken::NoValue"
---

# Typed causes in the engine layers

## Report

`refactor: carry a typed cause to every Internal and Corrupt` made crates/adtb/src/api/cause.rs the only builder of `RejectReason::Internal` and `OpenError::Corrupt` outside tests, with no `From<String>` for `Corruption`, and a search test with a self-test refusing direct construction. Left as text: `Broken::Write(String)` (write 44 sites, plan 12, exec 25), `Broken::Catalog(String)` (catalog 52), and `Broken::NoValue(String)`; `From` of `write::Error`, `exec::Error`, `catalog::Error` and the api `Error` let their `Internal(String)` through into `Corrupt`. Tests still asserting cause text: crates/adtb/src/catalog/tests.rs (3), write/delta/tests.rs (5). Direction: an enum per layer carrying the data its text is made from, then remove the three string variants; tests assert the variant.
