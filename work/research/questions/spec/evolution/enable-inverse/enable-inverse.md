---
title: "What the inverse of enable is"
pick: one
options:
  prior-state:
    title: "The operator that restores the prior state: disable from declared, deprecate from deprecated"
  always-disable:
    title: "Always disable"
---
Raised by the implementability review of the spec in docs/spec/.
`enable` moves a declared or a deprecated constructor to enabled, and
the spec gave `disable` as its inverse in both cases.

## spec/evolution/enable-inverse/prior-state

The inverse of every operator restores the prior state. The inverse of
an `enable` from declared is `disable`; the inverse of an `enable` from
deprecated is `deprecate`:

```adt
-- 0019 deprecate identities.Apple; 0020 enable identities.Apple
revert 0020   -- deprecate identities.Apple
```

## spec/evolution/enable-inverse/always-disable

The inverse is `disable` in both cases. Reverting the re-enable above
leaves `Apple` declared rather than deprecated, and is refused while
`Apple` has rows, which a deprecated constructor usually has.
