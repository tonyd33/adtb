---
title: SQL NULL does not mean what an Option None means
pick: any
raised-by: [interop/migrate/patterns/nullable-option]
options:
  translate-unique:
    title: A unique on a nullable column becomes a uniqueness over its Some values
  report:
    title: The tool lists every constraint whose meaning changes under the mapping
  live:
    title: Live with migrated constraints silently tightening
    excludes: [interop/migrate/null/translate-unique, interop/migrate/null/report]
---
In SQL `UNIQUE (email)` admits many NULLs, `CHECK (x > 0)` passes on
NULL, and `a = b` is unknown when either is NULL. `None` is an ordinary
value (`query/optional-sum/null-ctor`): two `None`s are equal and
collide under a unique. NULL also means "unknown" and "not applicable"
in one column; the tool cannot tell which.

## interop/migrate/null/translate-unique

```
constraint email_unique =
  from a in users, b in users
  where key a != key b
    and match a.email | Some _ -> a.email == b.email | None -> false
  select { a: a, b: b }
```

The SQL meaning, written out as a denial.

## interop/migrate/null/report

```
users.email  UNIQUE NULL       -> unique over Some only (translated)
orders.qty   CHECK (qty > 0)   -> holds on NULL in SQL; choose: Option or default
```

## interop/migrate/null/live

The migrated database rejects rows the old one accepted.
