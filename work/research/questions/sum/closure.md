---
title: Whether a sum's constructors are fixed by its declaration
pick: one
options:
  closed:
    title: Closed, the declaration lists every constructor
  open:
    title: Open, constructors may be added elsewhere
  owner-closed:
    title: Closed to the schema owner, open to consumers
---
Decides whether a query can know it has covered every case.

## sum/closed

`Provider` has exactly the constructors its `type` declaration lists.
Adding one is a schema change.

## sum/open

Constructors can be added by later declarations, like OCaml extensible
variants, Postgres `INHERITS` or Gel abstract types. Any union index
over the sum must be extended when a constructor is added. A match can never be known complete without a
wildcard.

## sum/owner-closed

The owner's queries see a closed sum; consumers in other schemas or
applications must treat it as open, as Rust `#[non_exhaustive]` does
across crates. egglog datatypes are the fully open precedent.
