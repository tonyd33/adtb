---
title: How locale order and case-insensitive comparison are expressed
pick: one
raised-by: [model/text]
options:
  sortkey-fn:
    title: Text orders by code point; a collation is a function to an ordered sort-key value
    needs: [btree/keyenc/lossy/code-point]
  typed:
    title: The collation is part of the text type
  column:
    title: A field declares a collation that its comparisons use
    links:
      - {rel: prior-art, to: prior/types-unicode}
---
German order, case-insensitive email uniqueness, accent-insensitive
search. `btree/keyenc/lossy` lists how an index stores a collated key.

## types/collation/sortkey-fn

```
add index (collate(name, "de-u-ks-level2")) in people
from p in people select p order by collate(p.name, "de")
type CiEmail = new Text            -- with a unique index on fold(email)
```

`collate` returns a `SortKey` (UTS #10 sort key, ordered by bytes);
`fold` is Unicode case folding. `==` and default order on `Text` stay
exact; every insensitive comparison is visible in the query.

## types/collation/typed

`Text @de`, `Text @ci`. Comparing `Text @de` with `Text @ci` is a type
error; `order` uses the type's collation. SQL collation derivation
rules, made static.

## types/collation/column

Postgres `name text COLLATE "de-x-icu"`, with nondeterministic
collations making `==` itself case-insensitive.
