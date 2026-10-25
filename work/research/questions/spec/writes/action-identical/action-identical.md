---
title: "Two retag delete actions on one type"
pick: one
options:
  identical-only:
    title: "Two retag actions reachable on one type must be the same action (same constructor, same fills); otherwise a schema error"
  last-wins:
    title: "The last one declared wins"
---
Raised while applying the ruling on how delete actions compose. A row
reached by two `retag` actions takes one new value, and the spec
refused only actions to different constructors, leaving two retags to
one constructor with different payloads undetermined.

## spec/writes/action-identical/identical-only

Every `retag` action among the fields of one type is the same action:
the same constructor with the same payload. Anything else is a schema
error when the type is declared.

```adt
type Login { id: Key logins, user: Key users on delete retag Revoked {} }
  = Active  { identity: Key identities on delete retag Revoked {} }
  | Revoked { at: Instant }
```

## spec/writes/action-identical/last-wins

The action of the last field declared applies. The outcome then
depends on field order, which nothing else in the schema makes
meaningful.
