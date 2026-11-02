---
title: A pinned conflict branch overwrites or deletes a row without using old
pick: one
raised-by: [evolution/old-readers/upsert-conflict/old-blocks]
options:
  names-old-blocks:
    title: Any branch naming old blocks enable, the target position included
  overwrite:
    title: Only a use of old's value blocks, so a target-only branch replaces an unseen row
---
A bundle pinned before `Link` runs this, and the key belongs to a `Link`:

```
insert notes.Plain { id: notes.key(1), body: "x" }
  unless conflict on key else update old set Plain { body: "x" }
```

The call types, since the branch needs nothing from the row, and it silently
destroys `url`, a field the bundle's author never knew. `set new` and `delete
old` do the same.

## evolution/old-readers/blind-overwrite/names-old-blocks

A branch naming `old` anywhere, `update old` and `delete old` included, reads
the whole relation variable. Only a branch leaving the row alone stays free:
`skip`, or a `fail`. The bundle blocks `enable r.C` until it is retired or
declares `hide new constructors`, under which the same call fails by key and
keeps the row. The rule is a syntactic occurrence, so it needs no analysis of
which fields a new value keeps.

## evolution/old-readers/blind-overwrite/overwrite

The bundle keeps working across `enable`, and a row of a constructor its
author never saw is replaced or deleted through it.
