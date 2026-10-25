---
title: When constraints are checked within a transaction
pick: one
options:
  commit:
    title: Every constraint checked at commit
  deferrable:
    title: Per constraint, at statement end or deferred to commit
    links:
      - {rel: prior-art, to: prior/constraint-sql-assertion}
  multi-assign:
    title: At statement end, with multiple assignment for compound changes
    links:
      - {rel: prior-art, to: prior/constraint-tutorial-d}
---
Inserting a `User` and its first `Identity` passes through a state
violating `has_identity`. Either the check waits or the two writes are
one statement. `constraint/at-least-one` was written as checked at
commit.

## constraint/timing/commit

No state inside a transaction is checked. A violation reports at commit,
far from the write that caused it; checks batch over the whole delta.

## constraint/timing/deferrable

```
constraint has_identity deferred = ...
unique (gid) in Google                -- immediate
```

SQL `DEFERRABLE INITIALLY DEFERRED`; Oracle assertions take both.

## constraint/timing/multi-assign

```
transact
  let u = insert users { name: "a" }
  let i = insert identities.Google { user: key u, gid: "g" }
  return key i
```

The Third Manifesto forbids deferred checking (unverified wording): the
block makes both inserts one statement, checked once after both.
