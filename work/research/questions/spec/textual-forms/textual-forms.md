---
title: "Where the textual forms of values with no literal are defined, and what they are"
pick: one
options:
  interop:
    title: "interop.md §1 owns them: NaN, inf, -inf, ISO 8601 times, P{m}M{d}D periods; a query writes them as T.parse calls"
  literals:
    title: "syntax.md gives each a literal token of the query language"
---
Raised by the reference interpreter. No part defined the textual forms
of NaN, the infinities, the time values, `Bytes` or `Period`.
syntax.md §2.5 and interop.md §1.1 pointed at each other for them, and
prelude.md §4 referred to forms syntax.md did not give.

## spec/textual-forms/interop

interop.md §1 states each form. A float's special values are `NaN`,
`inf` and `-inf`. `Instant`, `Date`, `Time`, `DateTime` and
`ZonedDateTime` print in ISO 8601 at microsecond precision, a zoned
value with its zone name in brackets. A `Period` is `P{m}M{d}D`. In
query source the same text is the argument of a `T.parse` call:

```adt fragment
Instant.parse("2026-10-25T00:00:00.000000Z")   -- : Option Instant
Period.parse("P1M0D")
F64.parse("NaN")
```

The textual syntax is then the literal syntax plus these forms, and
the lexer has no time or special-float tokens.

## spec/textual-forms/literals

Each special value gets a token: `nan`, `inf`, `2026-10-25T00:00:00Z`,
`P1M`. Values become writable without `Option`, but the lexer grows a
token for every time type, `inf` and `nan` become reserved words, and
a date token collides with integer subtraction (`2026-10-25`).
