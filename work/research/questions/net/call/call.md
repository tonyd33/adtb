---
title: How a call names a registered query
pick: one
raised-by: [lang/registered/named]
options:
  name:
    title: The query's name within the bound bundle
  hash:
    title: A content hash of the checked query
    links:
      - {rel: prior-art, to: prior/surface-persisted-queries}
  ordinal:
    title: A small id the engine assigned at registration, returned to codegen
---
No query text on the wire except in the ad hoc role.

## net/call/name

```
-> Call { query: "login_key", args: [uuid] }
```

Convex `module:fn`; readable in traces.

## net/call/hash

```
-> Call { query: #7f3c, args: [uuid] }
```

Two bundles with the same text share an entry.

## net/call/ordinal

```
$ adt register --bundle web-2.4 queries/
login_key  #7f3c  ordinal 3
-> Call { query: 3, args: [...] }
```

Codegen embeds the ordinal; one varint per call.
