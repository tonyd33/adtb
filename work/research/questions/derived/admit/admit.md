---
title: Which queries may be stored
pick: one
raised-by: [derived/materialize/storage-choice, derived/materialize/refresh-kind]
options:
  bounded:
    title: Refuse a stored query without an index-bounded incremental plan
    links:
      - {rel: same-as, to: constraint/admit/bounded}
  any-fallback:
    title: Admit any query; parts with no incremental plan are recomputed
    links:
      - {rel: prior-art, to: prior/derived-oracle-mv}
  declared:
    title: Admit unbounded stored queries only when marked so
    links:
      - {rel: variant-of, to: constraint/admit/declared}
---
Constraints admit only index-bounded checks (`constraint/admit/bounded`).
Stored views face the same cost question with the same compiler.

## derived/admit/bounded

```
store open_by_user
-- refused: no index invoices.Open (user) for the delta of users
```

The refusal names the missing index or operator, as for constraints.

## derived/admit/any-fallback

Oracle `REFRESH FORCE` falls back to complete refresh;
`DBMS_MVIEW.EXPLAIN_MVIEW` reports which methods apply.

## derived/admit/declared

```
store top_customers unbounded
```

The schema reports each stored view's maintenance class.
