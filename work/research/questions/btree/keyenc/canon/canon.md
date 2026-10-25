---
title: Values equal in the language must encode to identical bytes
pick: one
raised-by: [btree/keyenc/memcmp]
options:
  canonicalize:
    title: Encoding maps each equality class to one byte string
  total-order:
    title: Key equality is byte equality, so -0.0 and 0.0 are distinct keys
    links:
      - {rel: prior-art, to: prior/btree-key-encodings}
  live:
    title: Live with equal values that compare unequal in the index
    assessment: {grade: broken, why: a unique index admits two values the language calls equal}
---
IEEE floats have -0.0 = 0.0 and many NaNs; a unique index on a float
key or on a sum carrying one must agree with `=`. Postgres refuses
deduplication for float4 and float8 for this reason.

## btree/keyenc/canon/canonicalize

-0.0 encodes as 0.0, every NaN as one NaN; decoding returns the
canonical value, so the index cannot return the original sign.

## btree/keyenc/canon/total-order

FoundationDB: flip all bits if the sign bit is set, else flip the
sign bit; the IEEE total order, -0.0 < 0.0. The language's `=` on
floats is then this total order, not IEEE equality.

## btree/keyenc/canon/live

Accept the mismatch.
