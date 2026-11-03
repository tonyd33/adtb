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

## 2026-11-03: catalog slice landed

`refactor: carry a typed cause for every catalog Internal`. `catalog::Broken` (crates/adtb/src/catalog/broken.rs) has a variant per site group holding the data its text came from (ids, typed inner errors, the pending object's module, name and index id); `catalog::Error::Internal` and `change::Failed::Internal` hold `Box<Broken>`; `api::cause::Broken::Catalog` holds `catalog::Broken`. About 45 sites moved, none reclassified, no error changed kind (Internal, Corrupt, refusal), per review. Tests assert the variant: catalog/tests.rs three, api/server/tests.rs one. Full check 3633 passed.

Left for later slices:

- `catalog::Error::Corrupt(String)`: `load` turns `kept_indexes`' and `index_program`'s typed `Broken` back into text (`Corrupt(e.to_string())`, catalog/mod.rs ~775).
- `Broken::Admission(write::Error)` can only hold `write::Error::Internal(String)`, since `write::refusals` widened to `write::Error` to wrap `Unadmitted::Internal(String)`; typed in the write slice.
- `exec::Error::Internal` sites in catalog/rows.rs and keying.rs belong to the exec slice.
- Write (44 sites), plan (12), exec (25), `Broken::Write`, `Broken::NoValue`, as above.
