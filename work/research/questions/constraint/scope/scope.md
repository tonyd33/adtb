---
title: Where a row-level constraint attaches in a sum
pick: some
raised-by: [constraint/row-check, constraint/fd, constraint/exclusion]
options:
  ctor:
    title: On one constructor, over its own fields
  ctor-set:
    title: On a set of constructors, over fields they share
    links:
      - {rel: variant-of, to: index/ctor-set}
      - {rel: prior-art, to: prior/constraint-gel}
  match:
    title: On the sum, as an exhaustive match giving one predicate per constructor
---
Scope works as for indexes: a constructor set of one sum. The question
is whether a constraint over several constructors names the set or
splits by constructor.

## constraint/scope/ctor

```
check email_length on identities.EmailPass (length(email) <= 254)
```

## constraint/scope/ctor-set

```
check sub_present on identities.(Google | Apple) (length(sub) > 0)
constraint no_double_booking =
  from a in bookings.(Room | Desk), b in bookings.(Room | Desk)
  where a.resource == b.resource and overlaps(a.during, b.during) and key a != key b
  select { a: key a, b: key b }
```

Joint across the set: an exclusion over `Room | Desk` compares rows of
both, as Gel applies a supertype's `exclusive` across all subtypes; Gel
`delegated` is the per-constructor reading. Fields must be shared by
every member.

## constraint/scope/match

```
check login_valid on identities (match self
  | EmailPass e -> length(e.email) <= 254
  | Google g    -> length(g.gid) > 0
  | Apple a     -> true)
```

Checked exhaustive like any match, so every constructor states its
case, `true` included.
