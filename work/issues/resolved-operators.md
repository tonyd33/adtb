---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 709
summary: only the 10 declaring operators have a resolved form
closed: 2026-10-27
---

# Feature: resolved forms for the remaining operators

## Report

`Source::resolved` refuses every operator outside the 10 declaring ones (about 40), so a migration using one cannot be logged. Give each a resolved form and an arm in `catalog/oplog.rs` `operator`. Includes `functions` rows for stored views, and Wasm function and `wasm_modules` rows once `install` lands.

## 2026-10-27: done

Every operator now has a resolved form. `add default`, `drop default`, `store`, `unstore`, the three computed field operators, `grant`, `revoke`, `install`, `uninstall`, `adopt tzdb` and `script` resolve in `crates/adtb-lang/src/evolve/derived.rs` (`Resolver::derived`), and `crates/adtb/src/catalog/oplog.rs` (`derived`, `granted`) encodes them as `catalog::Operator` values. The catalog now has a `computed` row per computed field and a `relations` row of kind `Stored` per stored view; user relations are kind `Base` (they were `Catalog`).

Still open:

- `script` resolves, but `recordable` in `catalog/mod.rs` refuses it ("the engine does not run a `script` yet"): the catalog replay holds rows in memory and cannot run a write. Ignored test `a_script_writes_on_the_state_the_operators_before_it_produce` in `crates/adtb-sim/tests/oplog.rs`.
- `install` and `adopt tzdb` read their file from `Source::packages`, which the engine never fills (`install-package-bytes`), so the engine still refuses both. No `extensions`, `wasm_modules`, `tz_releases` or `zones` rows, and adopting a release changes no zone rules.
- `store` writes catalog rows only; the view is computed on read. Admission and maintenance stay with `stored-views`.
- Spec gaps: a grant or revoke of several privileges does not fit `Grant { privilege, target }`; the engine writes the privilege list as `privilege` and `target` as `None`. A default clause filling a field of several constructors creates several defaults, but `AddDefault` names one; the engine names the first.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the resops track.
