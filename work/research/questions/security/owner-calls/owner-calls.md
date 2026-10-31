---
title: Whether an owner calls with no call grant row
pick: one
options:
  bundle-and-schema-owner:
    title: The schema owner and the bundle's owner may call it; a family owner needs a grant for a bundle it does not own
  rows-only:
    title: Only a call grant row allows a call
---
security.md gives a bundle's owner the right to grant calls on it, and refuses a grant to oneself as a cycle.

```adt
-- r registered web-2.4 and holds no row on it
call web-2.4::q
```

## security/owner-calls/bundle-and-schema-owner

The bundle runs under its owner's rights already, so its owner calling it reaches nothing new. Rows only would leave an owner unable to run what it registered, with no way to grant itself. A family owner on a bundle the schema owner registered gets no such right: that bundle does not run under its rights.

## security/owner-calls/rows-only

A single rule, but `r` needs the schema owner to grant it its own bundle.
