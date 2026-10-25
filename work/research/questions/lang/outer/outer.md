---
title: What a left or full outer join gives for the missing side
pick: some
raised-by: [query/optional-sum/null-ctor]
options:
  option:
    title: The missing side is bound as an Option
  nested:
    title: No outer join; the related rows are a nested collection
    needs: [lang/results/nested]
    links:
      - {rel: prior-art, to: prior/lang-linq}
      - {rel: prior-art, to: prior/lang-nrc}
  these:
    title: A full outer join yields a sum of left, right or both
  none:
    title: No outer join; write the union of matched and unmatched
    excludes: [lang/outer/option, lang/outer/nested, lang/outer/these]
---
SQL pads the missing side with NULL. NULL is gone (`query/optional-sum/null-ctor`),
so the missing side needs a type. `query/outer-join` was defeated as the
lowering of `match`; this is outer joins as a user construct.

## lang/outer/option

```
from u in users left d in devices on d.owner == key u   -- d : Option Device
select { name: u.name, last: match d | None -> "never" | Some x -> x.seen }
```

One output row per unmatched user, as SQL; a user with two devices gives
two rows.

## lang/outer/nested

```
from u in users select { name: u.name, devices: from d in devices where d.owner == key u select d }
```

An empty collection replaces the padded row. LINQ's group join, NRC.
Flattening it back is an explicit generator.

## lang/outer/these

```
type These a b = This { l: a } | That { r: b } | Both { l: a, r: b }
from p in full prices, q in quotes on p.sku == q.sku   -- : These Price Quote
```

Haskell `these` package: the full outer join's three cases are a sum,
matched exhaustively. https://hackage.haskell.org/package/these

## lang/outer/none

```
(from u in users, d in devices where d.owner == key u select { name: u.name, last: Some d.seen })
union
(from u in users where not exists d in devices where d.owner == key u select { name: u.name, last: None })
```

Anti-join with `not exists`, no NULL needed.
