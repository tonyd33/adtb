---
created-at: 2026-11-16
status: open
kind: bug
priority: 150
summary: "a constraint or row check drops an unused deref through a stored row's reference, so a dangling key reports the reference constraint's Violated where the spec requires the check's fault"
tags:
  - lang
  - quality
  - class-none
---

# A constraint or row check over a dangling reference reports a violation instead of its fault

## Report

Found by the quality review of reviewed..main (c93c399e..92dbbc64) on main 92dbbc64.

In a constraint or a check clause, the checker gives the rows of a relation read, of `inserted(R)` (crates/adtb-lang/src/check/prelude.rs:758-760) and of a check's `self` (crates/adtb-lang/src/check/mod.rs:1258) `Provenance::Snapshot`. A reference field read from such a row becomes `Provenance::Reference` (crates/adtb-lang/src/check/expr.rs:967-971), and its deref is classed total (crates/adtb-lang/src/check/expr.rs:1036), so the normaliser drops it when its value is unused. A check runs over the state after the statement, where a reference may name no row: semantics.md promises a total deref only in a valid state, and constraints.md says a fault while evaluating a check takes precedence over every violation.

Repro with `work/tools/adtb-probe`, schema:

```
module test
type User = { id: Key users, name: Text }
type Phone = { id: Key phones, owner: Key users, seen: I64 }
relation users : User key id generate sequence
relation phones : Phone key id generate sequence
constraint c = from p in inserted(phones) where { a: p.owner.name, b: p.seen == 5 }.b select p
```

Text `insert phones from [Phone { id: phones.key(2), owner: users.key(99), seen: 1 }]`:

```
reference: refused with Faulted: NoSuchRow
engine: refused with Violated: constraint `test::phones.Phone.owner` is violated by 1 row
differ
```

The same split with `phones` in place of `inserted(phones)`, and with a row check `check pos on phones ({ a: self.owner.name, b: seen > 0 }.b)` in place of the constraint. A control that uses the deref's value (`p.owner.name == "zz"`) faults on both. The statement is refused either way; the refusal's kind and reason are wrong.

Fix direction: in `Context::Constraint` and in check clauses, give relation reads, `inserted()` and `self` a provenance that does not make a reference total, since the state a check reads is not known to satisfy its references. Add constraint and row check cases to conformance/queries/15-unused-deref.test.
