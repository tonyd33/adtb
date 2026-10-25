---
title: How nested result collections travel
pick: one
raised-by: [lang/results/nested]
options:
  inline:
    title: Each row carries its nested collections inline
  arrow-list:
    title: Arrow List of Struct columns, children buffers shared across the batch
    needs: [net/results/arrow]
  shredded:
    title: Each nesting level is its own stream keyed by parent position
    links:
      - {rel: prior-art, to: prior/lang-links}
---
`from u in users select { name: u.name, ids: u.<identities.user }` returns users
with their identities nested; a backlink is a collection
(`lang/nav/backlink-card/collection-typed`).

## net/nested/inline

```
<- Row { name: "ann", ids: [len 2][02 ...][03 ...] }
```

A large inner collection delays the next outer row.

## net/nested/arrow-list

`ids: List<DenseUnion<...>>`; offsets per level.

## net/nested/shredded

```
<- Stream 1 (users): [ann] [bob]
<- Stream 2 (ids):   [parent 0, ...] [parent 0, ...] [parent 1, ...]
```

The shredded form of nested query results (Cheney, Lindley, Wadler,
query shredding, SIGMOD 2014, unverified); each level streams
independently.
