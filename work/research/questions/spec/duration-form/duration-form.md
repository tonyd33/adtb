---
title: "The printed form of a Duration"
pick: one
options:
  largest-unit:
    title: "A DURATION literal in the largest unit that divides it exactly; zero is 0s"
  micros:
    title: "Always a count of microseconds: 90000000us"
  iso:
    title: "An ISO 8601 duration read by Duration.parse: PT1M30S"
---
Raised while writing the reference interpreter's printer. A `Duration`
has a literal (syntax.md §2.5), and interop.md §1.1 prints a value with
a literal in its literal syntax, but a duration has one literal per
unit (`90s`, `90000ms`), and printing needs one of them. A negative
duration has no literal: `-90s` is negation applied to `90s`.

## spec/duration-form/largest-unit

The count in the largest of `d`, `h`, `min`, `s`, `ms` and `us` that
divides the value exactly, with a leading `-` when negative; zero
prints as `0s`:

```adt fragment
-- 90 seconds prints 90s, 2 hours prints 2h, 1.5 seconds prints 1500ms
-- minus 90 seconds prints -90s
```

Short and read back by the literal grammar plus a sign, as integers
are. The reference interpreter prints this form.

## spec/duration-form/micros

Always microseconds. One rule and trivially exact, but long and hard
to read in error details and statistics.

## spec/duration-form/iso

ISO 8601 `PT1M30S`, read in queries as `Duration.parse("PT1M30S")`
like the other time values. Matches external tools, but a value with
a literal would print in another syntax, against interop.md §1.1, and
ISO 8601 durations allow calendar parts a `Duration` has not.
