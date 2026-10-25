---
title: Whether collections are values
pick: one
options:
  no-collections:
    title: No collection-valued fields and many-valued facts are child relations
  scalar-collections:
    title: Fields may hold collections of scalars only
    links:
      - {rel: prior-art, to: prior/datomic}
  nf2:
    title: Fields may hold collections of any value type including records and sums
    links:
      - {rel: prior-art, to: prior/lang-nrc}
      - {rel: prior-art, to: prior/third-manifesto}
---
First normal form or the nested relational model. NRC queries from flat
inputs to flat outputs are expressible in flat relational algebra
(conservativity; Paredaens and Van Gucht 1992, Wong 1996, unverified
citation), so nesting adds no power to flat queries.

## model/no-collections

```
type Tag = { doc: Key docs, tag: Text }
relation tags : Tag key (doc, tag)
```

## model/scalar-collections

`tags: Set Text`. Datomic `:db.cardinality/many`
(https://docs.datomic.com/schema/schema-reference.html), Postgres arrays.

## model/nf2

```
type Order = { lines: List { sku: Text, qty: Int, kind: Physical | Digital { url: Text } } }
relation orders : Order key id
from o in orders, l in o.lines where l.qty > 1 select l.sku
```

Third Manifesto relation-valued attributes; NRC (Buneman, Naqvi,
Tannen, Wong, TCS 149(1), 1995); DuckDB LIST of STRUCT.
