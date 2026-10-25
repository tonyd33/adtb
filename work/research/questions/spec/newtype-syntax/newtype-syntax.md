---
title: "Newtype construction and domains"
pick: one
options:
  call:
    title: "Email(\"a@b\") and type Email = new Text where p"
  juxtapose:
    title: "Email \"a@b\" and type Email = Text where p"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/newtype-syntax/call

Email("a@b") and type Email = new Text where p. Juxtaposition had no production and is ambiguous with nullary constructors.

## spec/newtype-syntax/juxtapose

Email "a@b" and type Email = Text where p. No grammar.
