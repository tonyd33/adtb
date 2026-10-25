---
title: A wildcard silently covers constructors added later
pick: any
raised-by: [query/wildcard]
options:
  warn-new:
    title: Warn when a wildcard covers a constructor added after checking
  live:
    title: Live with wildcards absorbing new constructors
    excludes: [query/wildcard-hides/warn-new]
---
`| _ -> null` written when `Provider` had two constructors also matches
`Apple`, exactly where a check would help.

## query/wildcard-hides/warn-new

Each stored query records the constructor set it was checked against.

## query/wildcard-hides/live

Wildcard means "anything else", by definition.
