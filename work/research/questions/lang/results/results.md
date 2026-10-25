---
title: The shape a query may return
pick: one
options:
  flat:
    title: Flat collections of records of scalars and sum values
  nested:
    title: Records may hold collections, nested to any depth
    links:
      - {rel: prior-art, to: prior/lang-links}
      - {rel: prior-art, to: prior/lang-edgeql}
      - {rel: prior-art, to: prior/lang-malloy}
---
Whether a query can return a user with their identities as one value, or
only rows the client regroups. Sum values in results are given by
`sum/value`.

## lang/results/flat

```
from u in users, i in identities where i.user == key u select { name: u.name, identity: i }
```

One row per pair; the client groups by user.

## lang/results/nested

```
from u in users
select { name: u.name, identities: from i in identities where i.user == key u select i }
```

NRC typing; query shredding (Cheney, Lindley, Wadler, SIGMOD 2014)
evaluates it as a fixed number of flat queries. EdgeQL shapes and Malloy
`nest:` are the same result form.
