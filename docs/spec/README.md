# adtb specification

Edition: draft, 2026-10.

adtb is a database whose data model has sum types at both the value
level and the relation level, with a query language of its own. This
directory is its specification: one file per part, each owning the
topics listed below. A part that needs another part's topic links to
it instead of restating it.

## Conformance

An implementation conforms when it meets every normative part and
passes the conformance suite. Where two sources disagree, the
conformance suite wins, then [semantics.md](semantics.md), then the
other normative parts, until the next edition corrects them.
The suite is written as scripts of statements and queries with
expected rows, and is released with each edition together with the
reference interpreter.

The normative parts state guarantees over constructor relations.
Heaps, pages, indexes and the file format are implementation, and the
annexes that describe them are informative.

## Editions

The spec is released in dated editions, each replacing the last. A
bundle names the edition it is written in, and every engine release
compiles every edition. `Error`, `Notice` and the prelude sums
([prelude.md](prelude.md)) are versioned with the edition, so a bundle
keeps matching the constructors of the edition it names. Catalog sums
evolve through the catalog log
([evolution.md](evolution.md#10-engine-releases)), not by edition.

## Parts

Normative:

| File | Owns |
| --- | --- |
| [model.md](model.md) | Values, base types, records, sums, newtypes, type parameters, relation variables, keys, references, equality, order and hashing |
| [prelude.md](prelude.md) | The prelude types (Bool, Option, Result, Tag, Uuid, Span, Rounding, Overflow, Fault, Principal, Position, TxnId, Report, Origin, Outcome, RejectedRow) and every prelude function, with signatures and faults |
| [syntax.md](syntax.md) | Lexical structure and the grammar of declarations, queries, writes and operators |
| [typing.md](typing.md) | Type checking, inference, subtyping, variance, generics, exhaustiveness |
| [queries.md](queries.md) | Meaning of comprehensions, `match`, patterns, navigation, grouping, aggregates, recursion, ordering, nested results, faults |
| [writes.md](writes.md) | Change sets, insert, delete, retag, upsert, delete actions |
| [constraints.md](constraints.md) | Constraints as denial queries, admission, checking at statement boundaries, transition constraints |
| [transactions.md](transactions.md) | Serializability, transact blocks, interactive transactions, retry, durability levels, idempotency |
| [evolution.md](evolution.md) | The operator log, schema operators and their inverses, versions, version translation, lifecycle stages, readers-first |
| [applications.md](applications.md) | Bundles, registered queries, version pinning, leases, retirement, the ad hoc role |
| [derived.md](derived.md) | Views, stored views, defaults, computed fields, time travel, patterns for valid time and event sourcing |
| [security.md](security.md) | Principals, authentication sources, grants, definer rights, audit, encryption at rest, tenancy |
| [wire.md](wire.md) | The server protocol |
| [embedding.md](embedding.md) | The in-process API |
| [catalog.md](catalog.md) | System relations, error sums, statistics, operational commands, configuration |
| [interop.md](interop.md) | Textual value syntax, external formats, loading, export, dump and restore, file scans, user-defined functions and aggregates |
| [semantics.md](semantics.md) | The kernel calculus and its denotational semantics; the surface is defined by desugaring to it |

Informative:

| File | Covers |
| --- | --- |
| [engine.md](engine.md) | Storage, file layout, MVCC, logging, indexes, buffer management, execution, replication |
| [implementation.md](implementation.md) | Crate structure, dependencies, unsafe code, testing and the reference interpreter |

## Conventions

- "Must" and "must not" state requirements on an implementation or on
  a program; anything else is description.
- Examples use the syntax of [syntax.md](syntax.md). A running schema
  of users and their sign-in identities is used throughout:

  ```adt
  module auth
  type User = { id: Key users, name: Text }
  type Identity { id: Key identities, user: Key users }
    = EmailPass { email: Text, hash: Bytes }
    | Google    { gid: Text }
  relation users : User key id generate sequence
  relation identities : Identity
    key id generate uuidv7
    unique (email) in EmailPass
    unique (gid) in Google
  set OAuth of identities = Google
  fn display_name (i: Identity) -> Text = match i
    | EmailPass e -> e.email
    | Google g    -> g.gid
  ```

  This is the schema at version 10. The canonical history continues
  with migration 0011, which adds `Apple { sub: Text }` joining
  `OAuth` with its `display_name` arm, and 0012, which enables it. Each part states the version
  its examples start from; examples in different sections are
  independent histories unless a section says otherwise. A part may
  extend the schema locally under new names, never by reusing a name
  above with another shape.
- Every module reads the prelude without qualification: `Bool`,
  `Option`, `Result`, `Tag`, `Uuid`, `Span`, `Rounding`, `Overflow`,
  `Fault`, `Principal`, `Position`, `TxnId`, `Report`, `Origin`,
  `Outcome`, `RejectedRow`, the error sums, and the functions listed in
  [prelude.md](prelude.md).
- Design rationale is not part of the spec. It lives in the design
  space, and each section ends with a "Rulings" line naming the rulings
  it states, for maintainers.
