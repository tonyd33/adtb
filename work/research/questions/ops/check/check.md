---
title: What an integrity check verifies
pick: any
raised-by: [buffer/faults/corrupt-detect]
options:
  structural:
    title: Every page checksum and B-link invariant, as amcheck
    links:
      - {rel: prior-art, to: prior/ops-amcheck}
  cross:
    title: Every live version has its index entries and every entry routes to an existing version of the constructor it names
    links:
      - {rel: prior-art, to: prior/ops-amcheck}
  logical:
    title: Every admitted constraint is re-run as its denial query over the whole database
  scrub:
    title: A background pass reads every page on a schedule so latent errors surface before they are needed
---
Checksums detect corruption on read; a page not read is not checked.
Settled constraints are queries that must return nothing
(`constraint/form/empty`), so a full logical check needs no second
checker: run each one.

```
check identities
from f in sys::check_findings select f
-- type Finding = Checksum { pid: PageId } | Order { index: IndexId, pid: PageId }
--              | Orphan { index: IndexId, ctor: CtorId, tid: Tid }
--              | Missing { ctor: CtorId, tid: Tid, index: IndexId }
--              | Violation { constraint: ConstraintId, witness: Text }
```

## ops/check/structural

amcheck `bt_index_check`; SQLite `quick_check`.

## ops/check/cross

amcheck `heapallindexed`; SQLite `integrity_check` index matching. In
a union index the constructor id in an entry must name the heap the
TID is in, and a retired constructor has no entries.

## ops/check/logical

Constraints were checked on each change set (settled delta
enforcement); a full run detects a wrong incremental check or
corruption that kept checksums valid. SQLite checks UNIQUE, CHECK, NOT
NULL; amcheck `checkunique`.

## ops/check/scrub

TigerBeetle scrubs its grid in the background (unverified detail);
ZFS scrub.
