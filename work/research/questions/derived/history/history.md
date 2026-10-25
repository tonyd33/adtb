---
title: How history is exposed in the language
pick: some
raised-by: [derived/time-travel/versioned, derived/time-travel/everything]
options:
  as-of:
    title: Whole queries evaluated at a past point
  versions:
    title: A relation of row versions with their validity range
    links:
      - {rel: prior-art, to: prior/derived-sql2011-temporal}
  changes:
    title: A relation of change sets, one per commit
    links:
      - {rel: prior-art, to: prior/derived-datomic-time}
---
One key spans every constructor (`model/one-key`), so a row's history
continues across a retag even though each version sits in a different
constructor relation.

## derived/history/as-of

```
at version 4120 { from i in invoices select i }
```

## derived/history/versions

```
from v in history(invoices) where v.row.id == 9
select { row: v.row, from: v.from, to: v.to }   -- v.row : Invoice, matched as a sum
```

SQL:2011 `FOR SYSTEM_TIME ALL`; Datomic `history`.

## derived/history/changes

```
from c in changes(invoices) where c.version > 4120 select c
-- c : { version: Version, inserted: Set Invoice, deleted: Set Invoice }
```

The change sets writes denote (`write/primitive/delta`), kept. Datomic
log API for time-only questions.
