---
title: What text equality compares
pick: one
raised-by: [model/text]
options:
  scalars:
    title: Text is a sequence of Unicode scalar values compared exactly
  nfc:
    title: Text is normalized to NFC on construction and compared exactly
    links:
      - {rel: prior-art, to: prior/types-unicode}
  canonical-eq:
    title: Stored as given, compared by canonical equivalence
    excludes: [types/eq/leibniz]
    links:
      - {rel: prior-art, to: prior/types-unicode}
---
`"é"` as U+00E9 and as U+0065 U+0301 are canonically equivalent:
same character, different scalars. Text is UTF-8 (`model/text`);
invalid UTF-8 is `Bytes`.

## types/text-eq/scalars

Rust `str`, Postgres with a deterministic collation. The two `é`
spellings are distinct keys and distinct set members.

## types/text-eq/nfc

```
insert users { email: Email "re\u{0301}sum\u{00e9}@x" }   -- stored as NFC
```

Every `Text` value is in NFC, so code-point equality is canonical
equivalence and the key encoding stays lossless. The bytes a client
sent are not kept; `Bytes` keeps them.

## types/text-eq/canonical-eq

Swift `String` equality. Keys encode the normalized form while the
heap keeps the original (`btree/keyenc/lossy/unpack-info` or
`heap-recheck`).
