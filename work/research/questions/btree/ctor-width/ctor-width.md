---
title: How many bytes the constructor id takes in an entry
pick: one
raised-by: [encoding/native-cost/union-index]
options:
  u16:
    title: Fixed 2 bytes
  u32:
    title: Fixed 4 bytes
    links:
      - {rel: prior-art, to: prior/pg-global-index}
  uvarint:
    title: Order-preserving varint, one byte for small ids
    links:
      - {rel: prior-art, to: prior/btree-key-encodings}
  local-code:
    title: Per-index dense code for the constructors in its set
---
Constructor ids come from a catalog counter that never goes back and
are never reused, so the largest id only grows, with every add, split
or aborted DDL. The id is paid once per entry (or per posting group,
see `btree/posting`). A Postgres index tuple is 8 bytes of header
(6-byte TID, 2-byte info) plus the key, MAXALIGNed to 8.

```
unaligned, id: Int8, u32 ctor:   key 8 | ctor 4 | tid 6       = 18 B + 4 B line pointer
unaligned, id: Int8, uvarint:    key 9 | ctor 1 | tid 6       = 16 B + 4 B line pointer
Postgres-style, MAXALIGN 8:      hdr 8 (holds tid) | key 8 | ctor 4 = 20, stored 24
```

Under 8-byte alignment a 1- and a 4-byte id cost the same.

## btree/ctor-width/u16

65536 constructors per database lifetime. Two bytes, fixed offset.

## btree/ctor-width/u32

Dilip Kumar's 2025 global index patch stores a 4-byte partition id per
entry, mapped through a catalog, not an OID.
https://www.postgresql.org/message-id/CAFiTN-uyec_y2QS2whUam8Rp1M%2BPPGuP0Zz45uX_U6mhQ8mtRg%40mail.gmail.com

## btree/ctor-width/uvarint

CockroachDB `EncodeUvarintAscending`: values up to 109 are the marker
byte itself (136 + v), larger ones a marker giving the byte count then
big-endian bytes, at most 9. Order-preserving and prefix-free, so it
can sit inside a memcmp key.
https://raw.githubusercontent.com/cockroachdb/cockroach/master/pkg/util/encoding/encoding.go

## btree/ctor-width/local-code

The index's metadata maps each constructor of its set S to a code
`0..|S|-1` in a byte; `S = {Google}` needs zero bytes. Adding a
constructor to a sum-wide index assigns the next code; retiring one
leaves its code unused.

```
key id on identities   codes: EmailPass 0, Google 1, Apple 2
entry: key | code u8 | tid
```
