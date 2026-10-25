---
title: How an embedded database migrates and registers
pick: one
raised-by: [engine/embedded-server]
options:
  open-applies:
    title: open() applies bundled migrations and registers the app bundle
  explicit:
    title: The application calls migrate and register itself
---
Embedded, the application ships its schema log and its bundle; there is
one reader process, so leases and readers-first are trivial.

## surface/open-applies

```
let db = adt::open("app.db", schema::LOG, queries::BUNDLE)?;
```

opens, applies missing operators, registers the bundle, and refuses to
open a file whose log has diverged from the bundled one.

## surface/explicit

`db.migrate(LOG)` and `db.register(BUNDLE)` as separate calls.
