---
title: How the ad hoc role is separated from application access
pick: some
raised-by: [surface/adhoc/scoped]
options:
  principal:
    title: Ad hoc text is accepted only from principals holding an ad hoc grant
  listener:
    title: Ad hoc text is accepted only on a separate listener
  session-bundle:
    title: Each ad hoc query is registered into a session bundle before it runs
    links:
      - {rel: same-as, to: surface/adhoc/no-repl/session-bundle}
---
Only this role sends query text, so the wire has a text call that
application principals never get.

## net/adhoc/principal

```
grant adhoc read identities to analyst
-> CallText { text: "from i in identities select i" }   -- Denied for role app
```

## net/adhoc/listener

`adt serve --adhoc unix:/run/adt/admin.sock`; reachable only from the
host or a bastion.

## net/adhoc/session-bundle

The session bundle pins a version and counts as a reader for
readers-first; results use per-call descriptors (`net/types/descriptor`)
since no codegen exists.
