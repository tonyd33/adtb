---
title: Whose rights a registered query runs with
pick: one
raised-by: [security/subject/queries]
options:
  definer:
    title: The bundle's rights; the caller needs only the call grant
    links:
      - {rel: prior-art, to: prior/security-pg-grant}
  invoker:
    title: The caller's rights; it needs every relation grant the query uses as well
    needs: [security/subject/relations]
  declared:
    title: Each query states definer or invoker
---
Postgres functions default to invoker; definer functions are how
Postgres and Supabase let a role reach rows it cannot read directly.

## security/rights/definer

```
bundle web-2.3 pins 9 (a91c...) owner web_owner
query my_invoices () -> Set Invoice = ...

grant call web-2.3 to web
```

The query is the capability. Revoking the bundle owner's read on a
relation fails the bundle's recheck like a schema change would.

## security/rights/invoker

A call of `my_invoices()` by `web` also checks `web`'s grants on every
constructor relation the query reads, so a call grant alone gives
nothing.

## security/rights/declared

```
query my_invoices () -> Set Invoice definer = ...
query report () -> Set Invoice invoker = ...
```
