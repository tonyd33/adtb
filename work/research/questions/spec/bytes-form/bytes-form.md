---
title: "The textual form of Bytes"
pick: one
raised-by: [spec/textual-forms/interop]
options:
  hex:
    title: "0x followed by two lowercase hex digits per byte"
  base64:
    title: "Base64 of the bytes"
---
The textual forms of interop.md §1 need one for `Bytes`.

## spec/bytes-form/hex

`0x` then two hex digits per byte, printed lowercase; a reader accepts
either case. The empty value is `0x`, so it never reads as an empty
CSV cell (which is `None` for an `Option`). Text order on equal-length
values matches byte order. In a query: `Bytes.parse("0x00ff")`.

## spec/bytes-form/base64

RFC 4648 base64, padded. A third shorter than hex, but a byte cannot
be read off the text, the empty value is the empty string and collides
with an empty CSV cell, and the alphabet holds `+` and `/`.
