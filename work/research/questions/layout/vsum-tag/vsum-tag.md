---
title: Width of a stored value-level sum tag
pick: one
raised-by: [sum/value]
options:
  u8:
    title: One byte with at most 255 constructors per sum
  u16:
    title: Two bytes
  varint:
    title: A variable-length integer
  min-bits:
    title: The fewest bits for the sum's tag space with tags of several fields bit-packed
---
Relation-level sums store no tag per row: the heap is the constructor,
and union index entries carry the catalog constructor id. Value-level
sums store one per value.

## layout/vsum-tag/u8

FlatBuffers union type field; DuckDB `UNION` caps at 255 members.
https://flatbuffers.dev/schema/
https://duckdb.org/docs/2.0/sql/data_types/union.html

## layout/vsum-tag/u16

Cap'n Proto discriminants. https://capnproto.org/encoding.html

## layout/vsum-tag/varint

Protobuf `oneof` field tags are varint keys, 1 byte below 16 field
numbers (unverified for oneof specifically).

## layout/vsum-tag/min-bits

`Bool`, `Status` (3 constructors) and a 5-constructor sum in one row
fit in one byte (1 + 2 + 3 bits). The width changes when a sum outgrows
its power of two, which is a format change.
