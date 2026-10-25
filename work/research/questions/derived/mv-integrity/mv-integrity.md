---
title: What a stored derived relation can carry
pick: any
raised-by: [derived/materialize/storage-choice, derived/materialize/refresh-kind]
options:
  indexed:
    title: Indexes over constructor sets of the stored result
    links:
      - {rel: gains, to: index/ctor-set}
  keyed:
    title: A declared key on the stored result, checked as a constraint
  fk-target:
    title: References into a stored derived relation
    needs: [derived/freshness/in-txn]
  constrained:
    title: Constraints read the stored relation and its delta
    links:
      - {rel: gains, to: constraint/aggregates/maintained}
---
A query result is a sum like a stored relation (`model/sum-results`),
so a stored view can carry what relations carry.

## derived/mv-integrity/indexed

```
store open_by_user
  index (open)
```

Materialize indexes views; Postgres indexes materialized views.

## derived/mv-integrity/keyed

```
store open_by_user key user
```

The key is a constraint on the query's result, checked through the
view's delta; or proved from the query (a projection of `users` key),
then never checked.

## derived/mv-integrity/fk-target

```
relation flags : { account: Key active_accounts, note: Text }
```

The referenced relation changes on writes to its base, so the FK is
checked on the view's delta inside the writing transaction; needs
`derived/freshness/in-txn`.

## derived/mv-integrity/constrained

`constraint over_limit = from r in open_by_user() where r.open > 5 select r`
checks the stored row's change instead of recounting.
