---
title: Whether a record type is a one-constructor sum
pick: one
options:
  record-is-ctor:
    title: A record type is a sum with one constructor
    excludes: [model/nominal-sums]
  ctor-payload:
    title: Records are their own types and a constructor carries a record payload
    links:
      - {rel: prior-art, to: prior/lang-machiavelli}
---
Type-level counterpart of `model/unify`. Either products and sums are one
declaration form, or a record type exists apart from any sum and a
constructor applies a tag to it.

## model/record-is-ctor

`type Point = { x: Int, y: Int }` is `type Point = Point { x: Int, y: Int }`.
A constructor's fields belong to the constructor; there is no record
type to name on its own. Haskell `data` (Haskell 2010 report sec. 4.2.1).

## model/ctor-payload

`{ x: Int, y: Int }` is a type; a constructor is a tag over a record.
`EmailPass e` binds `e : { email: Email, hash: Bytes }`, which can
be passed, projected and returned without its tag.

```
type Creds = { email: Email, hash: Bytes }
type Identity = EmailPass Creds | Google { gid: Text }
from i in identities select match i | EmailPass c -> c | Google g -> ...
```

OCaml inline records and polymorphic variants, Machiavelli records and
variants (Ohori, Buneman, Breazu-Tannen, SIGMOD 1989).
