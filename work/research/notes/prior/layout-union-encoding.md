---
title: Inline union encodings (Cap'n Proto, FlatBuffers, Rust)
---
Cap'n Proto: a union's members overlap in the struct's data and pointer
sections and a 16-bit discriminant selects the live one; fields are
stored XORed with defaults; little-endian 8-byte words. FlatBuffers: a
union is a u8 type field (0 = NONE) plus an offset to a table. Rust
enums: a tag sized to the variant count with payload sized to the
largest variant, and niche filling where a payload has invalid bit
patterns (unverified detail).
https://capnproto.org/encoding.html
https://flatbuffers.dev/schema/
https://doc.rust-lang.org/reference/type-layout.html
