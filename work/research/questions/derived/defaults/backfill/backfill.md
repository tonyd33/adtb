---
title: Adding a field fills existing rows
pick: one
raised-by: [derived/defaults/on-type, derived/defaults/on-relation]
options:
  same:
    title: The field's default also fills existing rows when the field is added
  separate:
    title: The add operator states its own fill; the default governs later inserts only
    links:
      - {rel: gains, to: evolution/smo}
---
`evolution/add` adds a field to a constructor; old rows need a value.
Postgres fills old rows from the column default (`atthasmissing`,
`kb/postgres.md`, unverified).

## derived/defaults/backfill/same

```
add field Account.region: Text = "eu"
```

One expression; a default reading `tx_time()` gives every old row the
operator's time.

## derived/defaults/backfill/separate

```
add field Account.region: Text fill (match a.country | ...) default "eu"
```

The fill may read the old row; the operator records it for its inverse.
