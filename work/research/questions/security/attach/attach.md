---
title: What a row policy attaches to
pick: one
raised-by: [security/policy/predicate]
options:
  ctor-set:
    title: A constructor set; each constructor relation is filtered by the policies of the sets it is in
    links:
      - {rel: variant-of, to: index/ctor-set}
  sum-match:
    title: The whole sum, written as an exhaustive match over the row
    links:
      - {rel: prior-art, to: prior/security-gel-policies}
---
A sum read is the disjoint union of its constructor relations, and
`match` is a union of arm queries. Filtering each constructor relation
before the union keeps the lowering unchanged.

## security/attach/ctor-set

```
policy own on invoices.(Draft | Sent) for read = row.owner == caller.user
policy public on invoices.Published for read = true
```

A new constructor states the named sets it joins
(`surface/ctor-set/growth/declare`), and with them its policies.

## security/attach/sum-match

```
policy read on invoices = match row
  | Draft d -> d.owner == caller.user
  | Sent s -> s.owner == caller.user
  | Published _ -> true
```

Lowered per arm into constructor filters. Adding a constructor breaks
the policy's match, so recompile forces a visibility decision. Gel
subtypes inherit every supertype policy.
