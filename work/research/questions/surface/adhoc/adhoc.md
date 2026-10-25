---
title: Whether unregistered queries can run
pick: one
raised-by: [evolution/breaks/invisible/registered]
options:
  registered-only:
    title: Only registered queries execute
    links:
      - {rel: prior-art, to: prior/surface-convex}
  scoped:
    title: Ad hoc queries only on a separate interactive or admin role
  open:
    title: Any connection may send query text, checked at execution
    assessment: {grade: hack, why: application code can bypass registration and with it the recheck}
---
Ad hoc text is checked against the schema current when it runs; a
schema change between runs is noticed only when it fails.

## surface/adhoc/registered-only

```
-> Execute { text: "from i in identities select i" }
<- Error: unregistered query
```

Relay safelists, Convex.

## surface/adhoc/scoped

```
$ adt repl --role analyst
adt> from i in identities select match i | EmailPass e -> e.email | Google g -> g.gid
```

Application roles may run only registered queries. Ad hoc results use
the same encoding; the exhaustiveness check still runs.

## surface/adhoc/open

As every SQL database.
