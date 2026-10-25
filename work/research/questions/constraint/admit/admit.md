---
title: Which constraints the schema refuses
pick: one
raised-by: [constraint/assertion]
options:
  any:
    title: Admit every constraint query; costly ones fall back to recheck
    needs-one-of: [constraint/enforce/recheck, constraint/enforce/ivm]
  bounded:
    title: Refuse a constraint without an index-bounded incremental check
    needs-one-of: [constraint/enforce/delta, constraint/enforce/ivm]
    links:
      - {rel: prior-art, to: prior/constraint-sql-assertion}
  declared:
    title: Admit unbounded constraints only when marked so
    needs-one-of: [constraint/enforce/recheck, constraint/enforce/ivm]
---
A constraint's check cost is known at schema time from its query and
the indexes present. The schema can refuse what it cannot check
cheaply, or say so.

## constraint/admit/any

Tutorial D: any relational expression. A constraint over an unindexed
join scans on every write to either side, silently.

## constraint/admit/bounded

```
constraint primary_owned = ...
-- refused: no index on identities (id) for the delta of users
```

Oracle 26ai refuses by syntax class. Here the refusal names the missing
index or the operator outside the class, and dropping an index a
constraint's delta needs is itself refused.

## constraint/admit/declared

```
constraint balanced unbounded = ...
```

The schema reports each constraint's cost class, as `explain` does for
queries; the marker makes a full recheck a visible choice.
