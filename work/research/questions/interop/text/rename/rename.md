---
title: Text written before a rename names the old constructor
pick: any
raised-by: [interop/text/literal, interop/text/json]
options:
  versioned:
    title: A text document states its schema version and the reader translates through the operator log
    links:
      - {rel: gains, to: surface/source/op-log}
  live:
    title: Live with old text failing to parse after a rename
    excludes: [interop/text/rename/versioned]
---
Constructor names carry no identity (`evolution/remove-rows/reserve`
defeat reason); the wire uses stable ids (`surface/sum-encoding/stable-tag`).
Text uses names, so `Facebook { ... }` in a dump taken before
`rename constructor Identity.Facebook to Meta` no longer parses.

## interop/text/rename/versioned

```
-- adtdb text, schema version 42
Facebook { fbid: "f-1" }
```

Rename is invertible (`evolution/rename`), so the reader maps names at
version 42 to stable ids, then to current names.

## interop/text/rename/live

Old text is re-exported after each rename.
