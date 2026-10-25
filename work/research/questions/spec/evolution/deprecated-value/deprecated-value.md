---
title: "When a write may store a value of a deprecated value-level constructor"
pick: one
options:
  same-path:
    title: "Only when an equal value is at the same field path of the pre-state row with the same key"
  same-field:
    title: "When the pre-state row with the same key holds any value of the constructor in that field"
  never:
    title: "Never, even when the value is unchanged"
---
Raised by the implementability review of the spec in docs/spec/. A
constructor `E` of a sum used as a field type may be declared or
deprecated, and a write that stores a value of it is refused with
`NotOpen` unless the write keeps a stored value. The spec did not say
what keeping a value means for a row whose field already holds some
other value of `E`.

## spec/evolution/deprecated-value/same-path

A write that stores a value `w` of `E` at field path `p` of a row is
allowed only when the pre-state row with the same key holds a value
equal to `w` at `p`. Any other write storing `E` is `NotOpen`:

```adt
-- Provider.Legacy { code: Text } deprecated; row 7 holds Legacy { code: "a" } in provider
update u in accounts where key u == accounts.key(7) set { u with name: "x" }   -- allowed
update u in accounts where key u == accounts.key(7)
  set { u with provider: Legacy { code: "b" } }                                  -- NotOpen
```

## spec/evolution/deprecated-value/same-field

The test of the earlier text: allowed when the pre-state row holds any
value of `E` in that field. Lets a write replace a deprecated value by
a new, different one of `E`, so new values of a closed constructor
keep entering the database.

## spec/evolution/deprecated-value/never

Every write that stores a value of `E` is refused, even when it copies
the row's own value. Every update of a row that still holds a
deprecated value fails, so the rows cannot be touched until a script
rewrites them.
