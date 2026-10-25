---
title: A definer query reads whatever its parameters select
pick: any
raised-by: [security/rights/definer, security/rights/declared]
options:
  caller-bound:
    title: The engine passes the authenticated caller as an implicit parameter no client can set
  policies-under:
    title: Row policies apply to the caller even inside a definer query
    needs: [security/policy/predicate]
  live:
    title: Live with it; each query checks the caller itself
    excludes: [security/rights/deputy/caller-bound, security/rights/deputy/policies-under]
---
`invoice (id: Key invoices)` under definer rights returns any invoice to
anyone holding the call grant. A confused deputy.

## security/rights/deputy/caller-bound

```
query my_invoices () -> Set Invoice =
  from v in invoices where v.owner == caller.user select v
```

`caller` is in scope in every registered query; its value comes from
the session (`security/assert`), never from a parameter.

## security/rights/deputy/policies-under

Definer rights decide which relations; policies, evaluated for the
caller, decide which rows.

## security/rights/deputy/live

Convex: every function calls `ctx.auth.getUserIdentity()` and checks by
hand.
