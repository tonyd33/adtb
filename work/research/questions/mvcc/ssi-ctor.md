---
title: Whether an index range lock remembers which constructors the scan read
pick: one
raised-by: [engine/ssi]
options:
  whole-range:
    title: The lock covers the range for every constructor in the index
  ctor-tagged:
    title: The lock carries the scanned constructor set, an insert conflicts only if its constructor is in it
---
A query over `identities.(Google | Facebook)` may scan an index over all
of `identities` when no index over exactly that set exists. Whether an
`EmailPass` insert into the scanned range is a conflict depends on the
lock.

```
relation identities : Identity
  key id
  index (user)
from i in identities.(Google | Facebook) where i.user == u select i.id
-- T2: insert identities.EmailPass { id: 11, user: u, ... }
```

## mvcc/whole-range

The T2 insert creates a rw-antidependency from the reader: a false
positive, possibly an abort. An index over exactly the query's set
avoids it.

## mvcc/ctor-tagged

The lock is `(index, range, ctors)`, ctors a bitmap over the sum's
constructor ids; the writer tests `ctor in ctors`. Promotion to a page
or index lock keeps the bitmap, OR-ing on merge. With HyPer-style
predicate logs the constructor test is one predicate conjunct like any
other (`mvcc/precision`).
