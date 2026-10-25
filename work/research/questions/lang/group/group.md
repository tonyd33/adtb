---
title: How grouping is written
pick: one
raised-by: [query/agg-scope/whole]
options:
  into:
    title: Group binds a key and the group's rows as a collection
    links:
      - {rel: prior-art, to: prior/lang-linq}
  compute:
    title: Group by keys and compute named aggregates only
    links:
      - {rel: prior-art, to: prior/lang-morel}
      - {rel: prior-art, to: prior/lang-prql}
  nest:
    title: No group primitive; distinct keys with nested aggregates
    needs: [lang/results/nested]
    links:
      - {rel: prior-art, to: prior/lang-nrc}
---
`query/agg-scope/whole` keeps arms row-level and makes per-constructor
aggregates a group by constructor, so grouping must reach the
constructor.

## lang/group/into

```
from i in identities group i by i.user into g
select { user: g.key, n: count(x for x in g), newest: max(x.created for x in g) }
```

`g` is a collection of the group's rows; any expression over it, nested
result included.

## lang/group/compute

```
from i in identities group i.user compute { n: count, newest: max over i.created }
```

Morel `group ... compute`, PRQL `group {..} (aggregate {..})`. Result
rows hold keys and aggregates only.

## lang/group/nest

```
from k in (from i in identities select i.user)
select { user: k, n: count(i for i in identities where i.user == k) }
```

NRC has no group operator; the planner recognises the pattern.
