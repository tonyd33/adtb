---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 199
summary: "the checker misorders constructor fields, loses view rows in ad hoc text and misjudges aggregates, nested inserts and update where"
branch: adtb-chk
---

# Bug: checker findings from the conformance suite

## Report

The interpreter is right in every case; the checker or the test is
wrong. Fix each and keep the directive:

1. `model/02-named-fields` (5 directives): `build_ctor` in
   `check/expr.rs` zips the written field names with values in
   declaration order, so `{ y: 2, x: 1 }` binds x to 2. Pair each value
   with its own name.
2. `queries/02-key-of:99`: `check_adhoc` starts from `Checker::new`,
   so a view's `fn_rows` computed in `bodies` are lost and `key` of a
   view row is refused. Keep `fn_rows` in `Program` and seed the ad hoc
   checker from it.
3. `queries/10-aggregate-bindings:63` and `10-key-preserving:45`: the
   pre-normalisation `merges_values` test refuses `sum` over a set
   literal and over a view call. Remove it; the rule is syntactic after
   normalisation and `analysis::aggregate` already applies it.
4. `writes/02-nested-insert:27` and `:38`: a write-level `if` branch
   and a binderless write match arm do not clear `once`, so a nested
   `key (insert ...)` is accepted there. Clear it for both branches and
   every arm.
5. `writes/03-where-without-in`: `update x where p set e` without `in`
   checks `p` after rebinding `x` to the replace binder, leaving it
   free; the interpreter panics. Check `p` against the row's variable,
   as `delete` does.
6. `queries/06-forms:107` is a wrong test: `C x` on a nullary
   constructor is admitted. Make it a query expecting `0` and `1`.
7. `writes/01-transact-return:25` expects `{}` for the unit value. It
   prints `()` (question `spec/unit-print`, point `spec6`): change the
   row and say so in `docs/spec/interop.md` §1.1.

`queries/11-fix:40` is [`fix-least-typing.md`](fix-least-typing.md).
