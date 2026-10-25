---
title: Naming the other transaction and its keys discloses what another role read or wrote
pick: any
raised-by: [ops/conflicts/detail, ops/conflicts/log]
options:
  redact:
    title: Keys and the other query are shown only when the receiver may read them
  admin-only:
    title: Full detail only in the system relation, readable by a monitoring role
  live:
    title: Live with it
    excludes: [ops/conflicts/leak/redact, ops/conflicts/leak/admin-only]
---
An application role learns that another bundle touched key 4410 of
`identities.Google`, which it may not be allowed to read.

## ops/conflicts/leak/redact

The error states constructor and role; the key range appears only if
the receiving role can read that constructor relation.

## ops/conflicts/leak/admin-only

The error carries an id; `sys::conflicts` holds the rest, readable by a
role with monitor privilege.

## ops/conflicts/leak/live

Every role sees full detail.
