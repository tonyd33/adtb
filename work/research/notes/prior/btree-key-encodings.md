---
title: Order-preserving key encodings
---
FoundationDB tuple layer: type code byte then payload, strings
0x00-terminated with 0x00 escaped as 0x00 0xff, nested tuples
terminated. CockroachDB: marker byte per type, varints with the length
in the marker, descending by complementing bytes. MyRocks: 4-byte
index id, sign-flipped big-endian ints, strings in 8-byte groups plus
a count byte, unpack info for lossy collations. SQLite: typed record
compare with key columns then rowid.
https://raw.githubusercontent.com/apple/foundationdb/main/design/tuple.md
https://github.com/facebook/mysql-5.6/wiki/MyRocks-record-format
