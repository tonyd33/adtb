---
title: "Whether an or-pattern alternative that can never match is allowed"
pick: one
options:
  refused:
    title: "Refused, as an unreachable arm is"
  allowed:
    title: "Allowed"
---
Raised while implementing the checker against docs/spec/. queries.md
§6.8 refuses an arm that can never be chosen, but says nothing of an
alternative of an or-pattern that can never match.

```adt
match i
  | Google _          -> 1
  | Apple _ | Google _ -> 2     -- the Google alternative matches nothing
  | EmailPass _       -> 3
```

## spec/pattern/redundant-alternative/refused

An alternative of an or-pattern is refused when it matches no value
left unmatched by the earlier unguarded arms and the earlier
alternatives of its own arm, as an unreachable arm is.

## spec/pattern/redundant-alternative/allowed

The alternative is accepted and never matches. The arm is reachable
through its other alternatives, and the dead alternative is silent.
