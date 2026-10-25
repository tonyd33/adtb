---
title: Whether the schema's reference graph may contain cycles
pick: one
raised-by: [fk/to-sum, fk/to-ctor, fk/to-subset]
options:
  deferred-check:
    title: Any reference graph and references are checked at the end of a unit holding the whole cycle
    needs-one-of: [constraint/timing/commit, constraint/timing/deferrable, constraint/timing/multi-assign]
  option-in-cycle:
    title: Every reference cycle passes through an Option field
    assessment: {grade: costly, why: "a required reference becomes optional and each cycle needs a second write"}
  acyclic-schema:
    title: The schema forbids reference cycles including self-references
    assessment: {grade: costly, why: trees of rows become separate edge relations}
---
With no NULL, `Employee { manager: Key employees }` or a `User` whose
`primary` identity references back to it cannot be inserted one
checked row at a time.

## model/deferred-check

```
transact
  let u = insert users { id: 1, primary: 9 }
  let i = insert identities.Google { id: 9, user: 1, gid: "1" }
  return key u
```

Constraints hold at the end of the unit `constraint/timing` chooses.
Gel offers `deferred restrict` on links
(https://docs.geldata.com/reference/datamodel/links).

## model/option-in-cycle

`manager: Option (Key employees)`: insert with `None`, then update. The
root of a tree is `None`, which is honest; a mandatory cycle is not.

## model/acyclic-schema

The schema's reference graph is a DAG; hierarchies are
`relation reports : Report` with
`type Report = { boss: Key employees, report: Key employees }`.
