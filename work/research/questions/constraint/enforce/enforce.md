---
title: How cross-row constraints are checked on write
pick: some
raised-by: [constraint/assertion, constraint/inclusion, constraint/cardinality, constraint/fd]
options:
  recheck:
    title: Evaluate the whole constraint query at each check
    assessment: {grade: costly, why: cost is the database size per write}
  delta:
    title: Compile each constraint into per-relation delta checks
    links:
      - {rel: prior-art, to: prior/constraint-simplification}
      - {rel: prior-art, to: prior/constraint-sql-assertion}
  ivm:
    title: Maintain each constraint's violation set as an incremental view
    links:
      - {rel: prior-art, to: prior/constraint-ivm}
---
Keys, FKs and at-least-one have fixed checks. A general constraint
needs a strategy; they stack, one per constraint by what it admits.

## constraint/enforce/recheck

Run `primary_owned` in full; any row refuses the commit. Correct for
any query.

## constraint/enforce/delta

Nicolas 1982 simplification. For `primary_owned` the compiler emits:

```
on insert or update u in users:   probe identities (id) for u.primary, test i.user == key u
on update i in identities (user): probe users (primary) for key i, test key u == i.user
on delete i in identities:        nothing (cannot create a violation)
```

Each check reads the written rows plus index probes. Efficient when
every body atom reachable from a delta has an index on the join field:
conjunctive denials with negated existentials, which covers FKs,
inclusion, FDs, exclusion and at-least-one. Oracle 26ai admits about
this class: no aggregates, outer joins or set operators.

## constraint/enforce/ivm

The violation query is a materialized view kept by delta rules; commit
fails when it is non-empty. Aggregates become maintained counters, so
`balanced` and `session_limit` cost a group update per write. DBSP
incrementalizes aggregation, recursion and stratified negation; state
is the view's intermediate results, held per constraint.
