---
title: Whether JSON documents are a value type
pick: one
options:
  none:
    title: No JSON type; JSON is only an import and export format
  opaque:
    title: A Json base type stored as binary, queried by path functions, with no integrity inside
    assessment: {grade: costly, why: "the schemaless escape hatch users reach for instead of sums"}
  value-sum:
    title: Json is a prelude recursive value-level sum
    excludes: [model/relation-recursion-only, model/no-collections]
---
Sums replace the jsonb-with-type-field pattern (`encoding/jsonb`
defeated: loses integrity on inner fields). Payloads from third
parties are genuinely schemaless.

## interop/json/none

A webhook body is `Text` or `Bytes`; parsing into a sum happens at
load (`interop/json-map`).

## interop/json/opaque

```
relation hooks : { id: I64, body: Json } key id
from h in hooks where h.body.path("event.type") == Some { value: "push" } select h
```

## interop/json/value-sum

```
type Json = Null | Bool { b: Bool } | Num { n: F64 } | Str { s: Text }
          | Arr { xs: [Json] } | Obj { kv: [(Text, Json)] }
```

Needs recursive value types and collections in values, both ruled out.
