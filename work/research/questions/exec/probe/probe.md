---
title: Arms of a match probe the same union index once each
pick: any
raised-by: [[query/union-all, encoding/native-cost/union-index]]
options:
  fuse:
    title: One probe of the union index, then route each entry to its arm
    links:
      - {rel: variant-of, to: query/native}
  live:
    title: Live with one probe per arm
    excludes: [exec/probe/fuse]
---
Lowered as a union of arm queries, a lookup by sum key becomes N
lookups that all descend the same B-tree and keep only entries of
their constructor.

```
from i in identities where i.id == id
select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub
-- union-all: 3 descents of identity_pk, each filters entry.ctor
```

## exec/probe/fuse

The planner recognises arms whose sources are the same union index
range and emits one index scan whose entries `(key, ctor, tid)` are
routed by `ctor`: fetch the tuple from heap `ctor`, run that arm. This
is a physical operator under the union-all semantics; it does not
change what a match means.

```
IndexScan identity_pk (id = id)
  -> Route by ctor { EmailPass: Fetch(email_pass) -> arm0
                     Google:    Fetch(google)     -> arm1
                     Apple:     Fetch(apple)      -> arm2 }
```

## exec/probe/live

Point lookups cost one B-tree descent per constructor of the sum.
