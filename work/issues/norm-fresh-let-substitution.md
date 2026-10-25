---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 90
summary: "norm treats Fresh as total and substitutes a let whose row draws a key, so the row and key u get different keys"
branch: adtb-lang2
tags:
  - lang
---

# Bug: a let bound to an inserted row draws its generated key twice

## Report

`norm` treats `Fresh` as total, so it substitutes a `Let` whose bound
row draws a key. `let u = insert users {...}` then draws one key for
the inserted row and another for `key u`. In the nested-insert case the
second key matches no row and the statement fails with a spurious
`Violated app::identities.user`. `docs/design/kernel.md` §4.1 says a
term evaluates as often as its structure says; substitution duplicates
the draw.

Three `adtb-reference` tests fail on it:

```
cargo test -p adtb-reference a_block_binds_inserted_rows_with_their_generated_keys
cargo test -p adtb-reference a_nested_insert_is_part_of_the_change_set
cargo test -p adtb-reference a_registered_transaction_runs_as_one_statement
```

Expected: each passes; `norm` keeps a `Let` whose bound term contains
`Fresh` and substitutes it nowhere.
