---
title: "How the unit value prints"
pick: one
options:
  parens:
    title: "The unit value prints as `()`"
  braces:
    title: "The unit value prints as `{}`"
---
Raised by the conformance suite. interop.md §1.1 says a record of an
anonymous record type prints only the braces, and semantics.md §2.2
makes the surface unit type `()` the kernel record type `{}`. The same
section says the textual form is a value's literal syntax, and the unit
literal is `()`. Equal values must print identically, so one form wins.

## spec/unit-print/parens

`()` is the literal, the type's spelling and every surface mention of
the value, so reading it back needs no special case.

## spec/unit-print/braces

`{}` follows the anonymous-record rule with no exception, but `{}` is
not a literal of the query language.
