---
title: How variable-length fields stay prefix-free in a byte key
pick: one
raised-by: [btree/keyenc/memcmp, btree/keyenc/hybrid]
options:
  escape-term:
    title: Escape 0x00 inside the value and end it with a terminator
    links:
      - {rel: prior-art, to: prior/btree-key-encodings}
  chunked:
    title: Fixed groups of bytes each followed by a count byte
    links:
      - {rel: prior-art, to: prior/btree-key-encodings}
---
In a composite or sum key a field's encoding must not be a prefix of
another value's, or `("a", "bc")` and `("ab", "c")` collide. A length
prefix breaks byte order (Graefe sec 3.4).

## btree/varlen/escape-term

FoundationDB: `0x00` becomes `00 ff`, value ends in `00`. CockroachDB:
`0x00` becomes `00 ff`, terminator `00 01`. Overhead one byte per zero
byte plus the terminator; text rarely holds zero bytes.

```
"foo\0bar"  ->  01 66 6f 6f 00 ff 62 61 72 00
```

## btree/varlen/chunked

MyRocks: 8-byte groups, each followed by a byte 1-8 (that many bytes
significant, last group) or 9 (full, more follow). 12.5% overhead,
no scanning for escapes, group-aligned comparisons.

```
"foobar"  ->  66 6f 6f 62 61 72 00 00 06
```
