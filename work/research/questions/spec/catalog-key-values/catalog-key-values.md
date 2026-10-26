---
title: "What value r.key(v) takes for a catalog or sys relation"
pick: one
options:
  declared:
    title: "The relation's declared key type: I64 for a catalog id, Bytes for catalog::wasm_modules"
  i64:
    title: "I64 for every catalog and sys relation"
---
Raised by checking commands over the catalog (`pin plan
catalog::plans.key(8812)`). A catalog key field is typed `Key
catalog::r` of its own relation and filled by the engine, so nothing
said what value `.key(v)` takes; `catalog::wasm_modules` is keyed by
the hash of a module's bytes.

## spec/catalog-key-values/declared

`.key(v)` takes the relation's declared key type: an `I64` where the
key is a catalog id or another engine-assigned number, and `Bytes` for
`catalog::wasm_modules`, whose key field is declared `hash: Bytes`:

```adt
from w in catalog::wasm_modules where key w == catalog::wasm_modules.key(h) select w.bytes
```

## spec/catalog-key-values/i64

Every catalog and sys key value is an `I64`. Wrong for
`catalog::wasm_modules`, keyed by a hash.
