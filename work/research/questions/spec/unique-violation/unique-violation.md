---
title: "Which reason a unique conflict reports"
pick: one
options:
  violated:
    title: "KeyViolation for the key only; any other unique reports Violated with that unique"
  key-violation:
    title: "KeyViolation for every unique"
---
Raised by the third review of the spec in docs/spec/.

## spec/unique-violation/violated

KeyViolation for the key only; any other unique reports Violated with that unique. One reason per constraint kind, as semantics states.

## spec/unique-violation/key-violation

KeyViolation for every unique. Blurs key and unique.
