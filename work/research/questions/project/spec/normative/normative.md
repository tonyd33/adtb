---
title: Which parts the spec makes normative
pick: some
options:
  model:
    title: Types, sums, relation variables, keys and references
  language:
    title: Syntax, typing including exhaustiveness, and query meaning
  writes:
    title: Change sets, constraint checking and retag
  transactions:
    title: Serializability, durability and the retry contract
  evolution:
    title: Schema operators, inverses and reader pinning
  wire:
    title: The binary wire protocol
  embed-api:
    title: The in-process API
  file-format:
    title: The on-disk file format
    links:
      - {rel: rivals, to: engine/guarantees-only}
  catalog:
    title: The catalog relations and their types
---
Normative parts bind every conforming implementation and every future
version of this one; informative parts may change freely. Settled:
the catalog is relations (`engine/catalog-relations`), new binaries
read old files (`layout/format-version/read-old`), the wire carries
never-reused constructor ids (`surface/sum-encoding/stable-tag`).

## project/spec/normative/model

```
type Identity = EmailPass { user: Key users, email: Text } | Google { user: Key users, sub: Text }
relation identities : Identity key id
```

## project/spec/normative/language

`from`, `match`, `group`, `fix`, nested results, patterns and their
coverage rule.

## project/spec/normative/writes

Insert, update, delete, upsert as change sets; checked once per
multiple assignment.

## project/spec/normative/transactions

The client contract of `engine/retry/retry-block`: a transaction may
fail with a serialization failure and be rerun; read-only transactions
on a safe snapshot never do.

## project/spec/normative/evolution

The operator set, each operator's inverse or declared loss, and when a
bundle's pinned version is retired.

## project/spec/normative/wire

Needed for clients in other languages; the codegen of
`surface/codegen/schema-types` targets it.

## project/spec/normative/embed-api

The Rust (and C) API of the embedded core: open, begin, run registered
query, commit.

## project/spec/normative/file-format

SQLite's file format is a documented contract, so other tools read
the file. Here it would bind heaps, union index and page layout, which
`engine/guarantees-only` leaves to the implementation.

## project/spec/normative/catalog

```
from c in catalog::constructors where c.sum == "Identity" select { name: c.name, id: c.id }
```
