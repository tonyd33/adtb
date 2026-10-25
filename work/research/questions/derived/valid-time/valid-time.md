---
title: Whether valid time is built in or a modelling pattern
pick: one
options:
  pattern:
    title: Valid time is a period field with an overlap exclusion constraint
    links:
      - {rel: gains, to: constraint/overlap/range-index}
  period:
    title: Relations declare an application period with portion updates and temporal keys
    links:
      - {rel: prior-art, to: prior/derived-sql2011-temporal}
  bitemporal-all:
    title: Every relation is bitemporal
    needs: [derived/time-travel/everything]
    links:
      - {rel: prior-art, to: prior/derived-xtdb}
---
System time is when the database learned a fact; valid time is when it
held in the world. SQL:2011 keeps them separate.

## derived/valid-time/pattern

```
type Price = { sku: Text, during: Period Instant, amount: Decimal 2 }
relation prices : Price
  key (sku, during)
  interval index (sku, during)
constraint no_overlap =
  from a in prices, b in prices
  where a.sku == b.sku and a.during != b.during and overlaps(a.during, b.during)
  select a
```

Accepted range-index exclusion covers keys. Changing a sub-period is a
query that splits rows.

## derived/valid-time/period

```
relation prices : Price key (sku, during without overlaps) period during
update p in prices where p.sku == s for portion of during (d1, d2) set { p with amount: 5 }
```

SQL:2011 `FOR PORTION OF` splits overlapping rows; Postgres 18 adds
`WITHOUT OVERLAPS` and `PERIOD` FKs.

## derived/valid-time/bitemporal-all

XTDB: `_valid_from`/`_valid_to` on every table, defaulting to now
until end of time; plain queries see valid time now.
