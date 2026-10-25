---
title: Order and limit only at the top make top-n per group impossible
pick: any
raised-by: [lang/order/result-only]
options:
  top-n:
    title: A top n by key operator yielding a set, usable at any depth
  ordered-nested:
    title: Nested collections may be ordered lists in results only
  live:
    title: Live with top-n per group done by the client
    excludes: [lang/nested-order/top-n, lang/nested-order/ordered-nested]
---
"Each user with their latest three posts" needs a limit inside a
nested result. Results are sets ordered only at the outermost level.

## lang/nested-order/top-n

```
from u in users
select { user: u, posts: top 3 by p.created_at desc
                         (from p in posts where p.author == key u select p) }
```

`top n by k` is a set: the n elements greatest by k, deterministic when
k is a key of the collection, refused otherwise. Order is not kept;
the client sorts if it wants order.

## lang/nested-order/ordered-nested

Result types may contain `List T` at any depth; stored relations and
query internals stay sets.

## lang/nested-order/live

Clients fetch and trim.
