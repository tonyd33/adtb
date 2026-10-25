---
title: "The type of a synthesizing match, if or union whose arms do not all synthesize"
pick: one
options:
  least:
    title: "The least type T such that every arm checks against T; none is an error"
  annotate:
    title: "An annotation is required whenever an arm cannot synthesize"
---
Raised while implementing the checker against docs/spec/. typing.md
§5.1 says a synthesizing `match`, `if` or `union` takes the join of
its arms' types, but some arms have no synthesized type: `None`, `[]`,
an integer literal, a lambda. Each takes its type from an expected
type, so there is no arm type to join.

```adt
if c then None else Some(1)
```

## spec/join/no-expected/least

The type of a synthesizing `match`, `if` or `union` is the least type
`T`, under the subtype order, such that every arm checks against `T`.
When there is no such type the program is refused. Arms that
synthesize contribute their types; arms that only check are accepted
at any `T` they check against.

```adt
if c then None else Some(1)        -- Option Int
match i | Google g -> g | Apple a -> a   -- Identity.(Google | Apple)
```

## spec/join/no-expected/annotate

Any arm that cannot synthesize makes the whole form need an expected
type, from a signature or an annotation. Refuses
`if c then None else Some(1)` and every `match` with a literal arm
outside a checked position.
