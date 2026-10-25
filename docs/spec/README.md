# adtb specification

Edition: draft, 2026-10.

adtb is a database whose data model has sum types at both the value
level and the relation level, with a query language of its own. This
directory is its specification: one file per part, each owning the
topics listed below. A part that needs another part's topic links to
it instead of restating it.

## Conformance

An implementation conforms when it meets every part and passes the
conformance suite ([conformance.md](conformance.md)). Where two sources
disagree, the conformance suite wins, then
[semantics.md](semantics.md), then the other parts, until the next
edition corrects them.

The spec states what users and clients observe, as guarantees over
constructor relations. How an implementation provides them (index
structures, storage, the file format, the log) is not part of it. The
reference implementation's contracts between its subsystems are in
[../design/](../design/README.md), which is not normative.

Rulings: spec/tiers/three-tiers

## Editions

The spec is released in dated editions, each replacing the last. A
bundle names the edition it is written in, and every engine release
compiles every edition. `Error`, `Notice` and the prelude sums
([prelude.md](prelude.md)) are versioned with the edition, so a bundle
keeps matching the constructors of the edition it names. Catalog sums
evolve through the catalog log
([evolution.md](evolution.md#10-engine-releases)), not by edition.

## Parts

| File | Owns |
| --- | --- |
| [model.md](model.md) | Values, base types, records, sums, newtypes, type parameters, relation variables, keys, references, equality, order and hashing |
| [prelude.md](prelude.md) | The prelude types (Bool, Option, Result, Tag, Uuid, Span, Rounding, Overflow, Fault, Principal, Position, TxnId, Report, Origin, Outcome, RejectedRow) and every prelude function, with signatures and faults |
| [syntax.md](syntax.md) | Lexical structure and the grammar of declarations, queries, writes and operators |
| [typing.md](typing.md) | Type checking, inference, subtyping, variance, generics, exhaustiveness |
| [queries.md](queries.md) | Meaning of comprehensions, `match`, patterns, navigation, grouping, aggregates, recursion, ordering, nested results, faults |
| [writes.md](writes.md) | Change sets, insert, delete, retag, upsert, delete actions |
| [constraints.md](constraints.md) | Constraints as denial queries, admission, checking at statement boundaries, transition constraints |
| [transactions.md](transactions.md) | Serializability, transact blocks, interactive transactions, retry, durability levels, commit positions, idempotency |
| [replication.md](replication.md) | Replicas and reads on them, writes at replicas, failover and fencing, leases and upgrades at replicas, embedded replicas |
| [evolution.md](evolution.md) | The operator log, schema operators and their inverses, versions, version translation, lifecycle stages, readers-first |
| [applications.md](applications.md) | Bundles, registered queries, version pinning, leases, retirement, the ad hoc role |
| [derived.md](derived.md) | Views, stored views, defaults, computed fields, time travel, patterns for valid time and event sourcing |
| [security.md](security.md) | Principals, authentication sources, grants, definer rights, audit, encryption at rest, tenancy |
| [wire.md](wire.md) | The server protocol |
| [embedding.md](embedding.md) | The in-process API |
| [catalog.md](catalog.md) | System relations, error sums, statistics, operational commands, configuration, the fault model, integrity and maintenance, release compatibility |
| [interop.md](interop.md) | Textual value syntax, external formats, loading, export, dump and restore, file scans, user-defined functions and aggregates |
| [semantics.md](semantics.md) | The kernel calculus and its denotational semantics; the surface is defined by desugaring to it |
| [conformance.md](conformance.md) | The conformance suite, its test format and how a test is judged |

## Scope

The engine is built for tens of threads of one application calling the
in-process API ([embedding.md](embedding.md)). It does not provide:

| Non-goal | Consequence |
| --- | --- |
| Distributed databases | No sharding and no distributed transactions; one database is one node with optional replicas ([replication.md](replication.md)) |
| SQL compatibility | No SQL dialect, no SQL front end, no Postgres or SQLite wire protocol |
| Several writing processes | One process owns a database file; others reach it through a server ([embedding.md §1.1](embedding.md#11-one-process-per-file)) |
| A columnar storage format | Analytic speed comes from execution |

Rulings: project/concurrency/app-threads, project/nongoals/distributed, project/nongoals/sql-compat, project/nongoals/multi-process, project/nongoals/columnar-store

## Conventions

- "Must" and "must not" state requirements on an implementation or on
  a program; anything else is description.
- Examples use the syntax of [syntax.md](syntax.md). A running schema
  of users and their sign-in identities is used throughout:

  ```adt fragment
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
