---
title: Whether sums take type parameters
pick: one
raised-by: [query/optional-sum/null-ctor]
options:
  user-params:
    title: Users declare parametric sums and fields instantiate them
  builtin-params:
    title: The engine supplies Option and other parametric types and users cannot declare more
  no-params:
    title: No type parameters and each optional field declares its own sum
---
With no NULL every optional field is a sum. Whether there is one
`Option a`.

## model/user-params

```
type Option a   = None | Some { value: a }
type Result a e = Ok { value: a } | Err { error: e }
type User = { email: Email, phone: Option Phone }
from u in users select match u.phone | None -> "-" | Some p -> show(p.value)
```

Storage monomorphizes per instantiation. A relation-level sum may be an
instantiation: `relation jobs : Result Job Failure key id`.

## model/builtin-params

`Option`, `List`, `Set`, `Map` are engine types; user sums are
monomorphic. Postgres arrays and DuckDB LIST and MAP are type
constructors users cannot add to
(https://duckdb.org/docs/sql/data_types/overview).

## model/no-params

```
type MaybePhone = NoPhone | HasPhone { phone: Phone }
```
