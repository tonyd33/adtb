---
created-at: 2026-11-03
status: resolved
closed: 2026-11-03
kind: bug
priority: 125
summary: "a rename that changes a composite key's field sort order is not permuted for a pinned bundle, so its inserted keys and key equality read the fields swapped"
---

# Pinned composite key field order

## Report

A composite key's value is laid out with its fields sorted by name. Found reviewing `fix: name every Prim in the translation analyses`: with `add type Pair = { a: Text, b: Text }`, `add relation pairs : Pair key (a, b)` and a bundle pinned before `rename field Pair.a to x`, a pinned `txn` inserting `{a: "1", b: "2"}` and returning `key p` yields `Record([Text("2"), Text("1")])` under the pinned form `(a, b)`, so the client reads a = "2", b = "1"; and `from p in pairs where key p == pairs.key(a: "1", b: "2") select p.b` answers `[]` where the pin answers `["2"]`. Wrong answers in normal use. Defect class: identity by position in the key record. Direction: translation maps a key value field by field identity, never by sorted position; a search over every rename of a composite key field that reorders the sort, for inserts, key equality, references and print.

## 2026-11-03: resolved

`fix: translate a pinned composite key by field across a rename` adds `Map::key_order(rel)` (translate/map.rs), the permutation between the pinned and current sorted layouts, `Ok(None)` when they match and `Loss::Field` for a key field with no counterpart. Translation (`Walk::key_of`, `Walk::key_value`) and the call boundary (`recode` in crates/adtb/src/api/pinned.rs, forward for arguments and back for results, recursing into each key field by its type) use it; an ordered comparison of a key whose layout, or a nested key's, or a sum or record key field's order changed is refused naming the rename (`Loss::KeyOrder`). Fixed beyond the report, same cause: `value(key p)`, a key parameter, a reference built from a key literal refused by its foreign key, a key nested in a key. Search tests in crates/adtb-sim/tests/bundles.rs (two and three field keys, every rename of one key field to before, between and after the others, bodies for result, literal equality, whole `value(key p)`, parameter, reference, nested result and argument; floors 58 and 114), `a_pinned_key_value_reads_by_field_across_two_renames_that_reorder_its_fields`, and the translate search for ordered comparisons (30 translated, 24 refused). Five mutants fail by name. Full check 3631 passed.

Left: the engine search takes 1.59 s alone and 3.02 s under the full suite, most of it opening a database of 24 relations; split by key shape if it grows. The `Loss::Field` path of `key_order` is unreached, since a rekey is refused before the walk. `recode` keeps an `expect` stated as a precondition: only translated callables reach it.
