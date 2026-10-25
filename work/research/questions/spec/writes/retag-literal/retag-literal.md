---
title: "What a retag action's payload may contain"
pick: one
options:
  nullary:
    title: "Scalar literals and nullary constructors"
  scalar:
    title: "Scalar literals only"
---
Raised by the writes checker. writes §7.1 says the payload of `on
delete retag C { ... }` is a record of literals, and did not say
whether a nullary constructor such as `None` is one.

## spec/writes/retag-literal/nullary

A literal in the payload is a scalar literal or a nullary constructor:

```adt fragment
type Login { id: Key logins, user: Key users on delete cascade }
  = Active  { identity: Key identities on delete retag Revoked { note: None } }
  | Revoked { at: Instant, note: Option Text }
```

## spec/writes/retag-literal/scalar

Only scalar literals. A field of a value-level sum, `Option` included,
then needs a default to be filled, though its value is as constant as
a literal.
