---
title: How a sum and its constructors are declared
pick: one
options:
  alternatives:
    title: One declaration listing constructors separated by bars
  block:
    title: A block with one nested declaration per constructor
  scattered:
    title: Constructors declared separately, each naming its sum
---
Sums are closed, so every constructor is known where the schema is
read. The question is whether that set is visible in one place.

## surface/sum-decl/alternatives

```
type Identity = EmailPass { email: Text, hash: Bytes }
              | Google    { gid: Text }
```

Keys and uniques belong to relation variables (`surface/extent`), not to
the type.

ML and Haskell data declarations
(https://ocaml.org/manual/latest/typedecl.html).

## surface/sum-decl/block

```
sum Identity {
  key id
  ctor EmailPass { email: Text unique, hash: Bytes }
  ctor Google    { gid: Text unique }
}
```

Rust `enum`; leaves room for sum-level fields, keys and indexes in the
same block.

## surface/sum-decl/scattered

```
sum Identity key id
ctor EmailPass of Identity { email: Text unique, hash: Bytes }
ctor Google    of Identity { gid: Text unique }
```

Gel `type Google extending Identity`, Third Manifesto subtypes. The
set is closed by the schema as a whole, not by the declaration.
