---
title: Constraints, errors and statistics reveal rows a policy hides
pick: any
raised-by: [security/policy/predicate, security/grain/ctor-set]
options:
  total-ops:
    title: Expressions cannot fail on row values; partial operations return a sum
  barrier:
    title: Policy filters run before any expression that can fail, as security barriers
    links:
      - {rel: prior-art, to: prior/security-pg-rls}
  aligned-keys:
    title: A unique key or FK target on a policy-bound set must include the policy's discriminating fields
  opaque-errors:
    title: A constraint violation on a row the writer cannot read reports no key or constructor
  hidden-stats:
    title: Statistics, plans and conflict details are shown only to roles that may read every row they cover
    links:
      - {rel: prior-art, to: ops/conflicts/leak}
  live:
    title: Live with it
    excludes: [security/leak/total-ops, security/leak/barrier, security/leak/aligned-keys, security/leak/opaque-errors, security/leak/hidden-stats]
---
Postgres: unique and FK checks bypass RLS, "covert channel". SQL Server:
`1/(salary-100000)` reveals a filtered salary by a divide-by-zero
error. A retag into a hidden constructor reads to the writer as a
delete. `insert identities.Google { gid: g }` failing on uniqueness says
`g` exists.

## security/leak/total-ops

```
from p in payroll select p.total / (p.salary - 100000)   -- : Option Decimal
```

Division, overflow and casts yield `None` or an error constructor in
the value; no row makes the statement fail. Leakproofness is a type
property, with no per-function flag.

## security/leak/barrier

Postgres `security_barrier` and `LEAKPROOF`: only leakproof
expressions are pushed below the policy filter; the fastest plan may be
rejected.

## security/leak/aligned-keys

```
policy tenant on docs for read = row.tenant == caller.tenant
relation docs : Doc key (tenant, slug)       -- admitted
relation docs : Doc key slug                 -- refused: key crosses policy
```

Checked at schema time beside `constraint/admit/bounded`: a uniqueness
collision can only be with a row the writer may see.

## security/leak/opaque-errors

The error says the change set violated a constraint, with one bit left.

## security/leak/hidden-stats

SQL Server restricts `DBCC SHOW_STATISTICS` on RLS tables. Plan row
estimates and explain analyze counts are filtered or withheld.

## security/leak/live

Postgres documents the channel.
