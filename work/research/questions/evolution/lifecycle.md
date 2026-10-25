---
title: Whether enable, deprecate and retire are operators in the log
pick: one
raised-by: [surface/registration/order/pinned]
options:
  log-ops:
    title: enable, deprecate and retire are operators with versions like any other
  admin:
    title: They are administrative commands outside the log
  log-schema-only:
    title: enable and deprecate are log operators; retire is a deployment command outside the log
---
Adding a constructor is declared, then enabled for writes; removing one
is deprecated first; a stale bundle is retired. None has a defined home.

## evolution/log-ops

```
0011 add constructor Identity.Apple { sub: Text }
0012 enable identities.Apple
```

Each bumps the version, so a bundle pinned at 11 knows `Apple` exists
but cannot be written, and one at 12 may write it.

## evolution/admin

Commands change catalog flags without a version.

## evolution/log-schema-only

enable and deprecate change what a version means for every reader, so
they are versioned. Retiring a bundle changes deployment state only;
in the log it diverges a file from the migrations its program ships.
