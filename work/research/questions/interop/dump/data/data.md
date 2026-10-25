---
title: How a logical dump encodes rows
pick: some
raised-by: [interop/dump/logical]
options:
  text-rows:
    title: One row per line in the textual value syntax, per constructor relation
  arrow-ipc:
    title: An Arrow IPC stream per constructor relation
    links:
      - {rel: gains, to: exec/batch/arrow}
  wire:
    title: Blocks in the client wire encoding, with descriptors
    links:
      - {rel: prior-art, to: prior/interop-gel-dump}
---
Each constructor relation has one schema, so each dumps as one
homogeneous stream.

## interop/dump/data/text-rows

```
data identities.Google
{ id: 7, user: Key users 3, gid: "g-123" }
```

Diffable and greppable; large and slow to parse.

## interop/dump/data/arrow-ipc

Readable by every Arrow tool; one constructor heap is one schema with
no validity bitmap. Value-level sums are dense unions.

## interop/dump/data/wire

Gel dumps data blocks in its binary protocol encoding with type
descriptors in the header (https://docs.edgedb.com/database/reference/dump_format).
One encoder for clients and dumps.
