---
title: "Which match an arm clause extends"
pick: one
options:
  scrutinee-set:
    title: "Every match in the named object whose scrutinee's constructor set contains the new constructor at the new version; exactly one"
  over-type:
    title: "The one match over the type"
---
Raised by the implementability review of the spec in docs/spec/. "The
one match over the type" does not say whether a match over a named set
of the type counts, or a match over another instantiation.

## spec/evolution/arm-target/scrutinee-set

An `arm` clause targets every match in the named function whose
scrutinee's constructor set contains the new constructor at the new
version; it is refused unless there is exactly one:

```adt
-- display_name matches once over identities.OAuth and once over the whole Identity
add constructor Identity.Apple { sub: Text }
  joins identities.OAuth
  arm display_name: Apple a -> a.sub   -- refused: two matches would need the arm
```

A match over `identities.(EmailPass | Google)` written as a list does
not count, since the new constructor never joins it.

## spec/evolution/arm-target/over-type

The clause targets the one match whose scrutinee has the type. A match
over a list that does not gain the constructor counts and makes the
clause ambiguous, and one over a set that gains it is missed.
