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

## 2026-11-03: exec slice landed

`refactor: carry a typed cause for every exec Internal`. `exec::Broken` (crates/adtb/src/exec/broken.rs, 38 variants, Display rebuilding each old text) replaces `exec::Error::Internal(String)`, including the sites in catalog/rows.rs, catalog/keying.rs and write/past.rs; `write::Error::Exec` and `api::cause::Broken::Exec` carry it, and write's `exec::Error::OtherVersion` conversion now maps to `Exec(Broken::OtherVersion)`. No error changed kind or api outcome, and Display text is identical at every site, per review. exec/tests.rs and catalog/tests.rs assert variants. Full check 3633 passed.

Left from the review:

- No test reads `exec::Broken`'s Display: a mutant setting one arm's text survives every test. Add one value per variant against its expected text, or drop the custom texts the day Debug is derived.
- `exec::Broken`'s Debug prints its Display text quoted, so `{e:?}` details in `catalog::Broken` (`Validation`, `Map`, the `IndexKey` fallback) and keying's refusal read as before. Have those arms Display `exec::Error::Internal(b)` with `{b}`, then derive Debug.
- `Broken::OtherVersion` is a deployment binding mismatch, not a broken invariant, against the docs of `Broken` and `write::Error::Exec`, and duplicates `exec::Error::OtherVersion(String)`.
- `a_primitive_of_mistyped_operands...` accepts any operand as `found`.

## 2026-11-03: write slice landed

`refactor: carry a typed cause for every write Internal`. `write::Broken` (crates/adtb/src/write/broken.rs, derived Debug, Display keeping the old texts, one variant per site group) replaces `write::Error::Internal(String)`, `delta::Unadmitted::Internal(String)` and `api::cause::Broken::Write(String)`; `catalog::Broken::Admission` holds it. `write::detail` keeps the old `Internal("text")` form at the user details that printed a write error's Debug. The engine-relation failure in api/server/adhoc.rs is internal now, not `Invalid`. `TermKind` compares `Kind`s by Debug text, a true equivalence, since `Kind` holds an f64. write/broken_tests.rs reads every variant's Display through a `match` with no wildcard. No error changed kind or api outcome, per review. Full check 3636 passed.

Left from the review:

- Derived Debug of a `catalog::Error` holding a write cause now prints its structure where it used to print `Internal("text")`: api/cause.rs `Broken::CatalogError`, api/verify.rs `in_kept_transaction`, api/reindex.rs report rows, `write::Broken::Denials` and `Expiry`. None found reachable through normal use. Display the cause at those sites.
- No test pins the text of a write `Internal` inside `Corruption::Write`, the reindex failure cause or verify's details: reverting `Corruption::Write` to `{e:?}` passes every test.
- `RefusalUnrecorded` and `WitnessDenialUnrecorded` hold `api::Error`, so write names a layer above it.
- Relation names held as `String` where the site has a `RelId`, since Display has no schema: `UnstoredIndex`, `NotAnIndex` (reindex.rs), `UniqueUnindexed`, `UndeclaredIndex`, `KeyNotFields` (schema.rs), `ReferenceUnindexed` (eval.rs).
- Next slices: plan (`plan::Error::Internal`), `Broken::NoValue`, `catalog::Error::Corrupt(String)`, the exec Debug and Display items above.
