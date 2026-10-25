---
title: Which types have ==
pick: one
options:
  all-first-order:
    title: Every value type has a derived structural ==
  opt-in:
    title: A declared type has == only when it says so
---
Base types, sums, records, `Key S`, and nested collections in results
(`lang/results/nested`) all reach set deduplication. Function values
from `lang/abstraction/higher-order` are normalised away and never
stored.

## types/eq-types/all-first-order

Sums compare by constructor then fields, records field-wise, `Key S`
by key value, a nested collection as a set. No declaration can opt
out, so a stored or result type is always deduplicable.

```
type Status = Draft | Sent { at: Instant }
from o in orders select { status: o.status }   -- needs == on Status
```

Standard ML equality types, without the restriction to a separate
type-variable kind (http://sml-family.org/sml97-defn.pdf, unverified).

## types/eq-types/opt-in

```
type Blob = { data: Bytes } derive (Eq)
```

Rust `#[derive(PartialEq)]`. A relation of a type without `==` cannot
be a set, so stored and result types would need it anyway.
