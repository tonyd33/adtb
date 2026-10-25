---
created-at: 2026-10-25
status: open
kind: bug
priority: 95
summary: "decimal literals of another scale, integer literals against Decimal s, and div over decimals are refused though the spec accepts them"
tags: [lang]
---

# Decimal literals are refused against decimals of another scale

## Report

Found by the reference interpreter. The checker refuses three cases the
spec accepts:

- `1.0 == 1.00` and `1.5 > 1.49`, with "a literal with 2 fractional
  digits does not check against Decimal 1". `docs/spec/model.md` §3.3
  says decimal comparison is numeric across any scales.
- `x + 1` with `x: Decimal 0`. `docs/spec/typing.md` says an integer
  literal checks against any `Decimal s`.
- `div(2.0, 3, 4, HalfUp)`.

Expected: all three check.
