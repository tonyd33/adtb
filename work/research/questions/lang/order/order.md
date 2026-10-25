---
title: Where ordering lives
pick: one
options:
  result-only:
    title: Order and limit only at the outermost result
    links:
      - {rel: prior-art, to: prior/third-manifesto}
  typed:
    title: Ordering yields a list type usable anywhere
    needs-one-of: [lang/collection/lists, lang/collection/kinded]
    links:
      - {rel: prior-art, to: prior/lang-nrc}
      - {rel: prior-art, to: prior/lang-morel}
  none:
    title: Results unordered; clients sort
---
Whether `order by` and `take` are query operators that compose, or a
presentation step on the final result. Top-N per group needs ordering
inside a group.

## lang/order/result-only

```
from u in users select u order by u.name take 20
```

Allowed only as the last clauses; a subquery is unordered. Tutorial D
treats `ORDER` as outside the relational algebra.

## lang/order/typed

```
from d in depts
select { dept: d, top: from e in employees where e.dept == key d
                       order by e.salary desc take 3 }   -- top : List Employee
```

`order` turns a bag into a list; `take` is a list operator. Ordering
inside nested results and per-group top-N are ordinary expressions.

## lang/order/none

The engine never promises order; pagination is by key range in a
`where`.
