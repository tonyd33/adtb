---
title: A pipeline stage cannot name an earlier stage's binder
pick: any
raised-by: [lang/family/pipeline]
options:
  row-binders:
    title: Each stage binds its row under a name visible downstream
  live:
    title: Live with only the current row's columns in scope
    excludes: [lang/family/pipeline-scope/row-binders]
---
In PRQL a stage sees the columns of the relation flowing in. After a
join or a `select`, the original rows are gone, so a match arm's binder
cannot be reused two stages later, and correlated subqueries have
nothing to correlate on.

## lang/family/pipeline-scope/row-binders

```
from i = Identity
| join u = User (i.user == u)
| select {u.name, match i | ...}
```

Binders accumulate as in a comprehension, which is the comprehension
read top to bottom.

## lang/family/pipeline-scope/live

Recompute or carry columns forward explicitly.
