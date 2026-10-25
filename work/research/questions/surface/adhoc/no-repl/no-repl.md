---
title: No interactive exploration without ad hoc queries
pick: any
raised-by: [surface/adhoc/registered-only]
options:
  session-bundle:
    title: The REPL registers each query in a bundle dropped at session end
  live:
    title: Live with no REPL
    excludes: [surface/adhoc/no-repl/session-bundle]
---
Operators debugging production and developers exploring data need to
type queries.

## surface/adhoc/no-repl/session-bundle

```
adt> from i in identities.Google select i.gid
registered #a01 in bundle repl-tony-1 (expires at disconnect)
```

The REPL is a client like any other. A schema change during the
session rechecks and reports its queries; readers-first counts it as a
reader.

## surface/adhoc/no-repl/live

Every query goes through a build step.
