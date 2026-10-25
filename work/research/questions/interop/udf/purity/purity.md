---
title: Host code can be nondeterministic or effectful
pick: any
raised-by: [interop/udf/wasm, interop/udf/host-fn, interop/udf/native-plugin]
options:
  declared:
    title: Signatures declare determinism; only deterministic functions may appear in keys, indexes and constraints
  wasm-det:
    title: WebAssembly modules get no imports and canonical NaNs, so they are deterministic by construction
    needs: [interop/udf/wasm]
  live:
    title: Live with host functions that may differ between calls
    excludes: [interop/udf/purity/declared, interop/udf/purity/wasm-det]
---
An index key, a constraint check, a replayed change set
(`buffer/test/dst`), a replica and the normaliser
(`lang/abstraction/higher-order`) all assume equal inputs give equal
outputs. A function reading the clock or a random source breaks each.

## interop/udf/purity/declared

```
fn now_bucket () -> Instant volatile = host "now_bucket"
add index (geohash(lat, lon, 6)) in events    -- ok: geohash is deterministic
add index (now_bucket()) in events            -- refused: volatile
```

Postgres `IMMUTABLE`/`STABLE`/`VOLATILE`, trusted, not verified
(unverified).

## interop/udf/purity/wasm-det

With no WASI imports, fuel metering and NaN canonicalisation, a Wasm
function is a pure function of its arguments; wasmtime fuel is
deterministic (https://docs.wasmtime.dev/api/wasmtime/struct.Config.html).
The engine can run it in simulation and replay.

## interop/udf/purity/live

Indexes on such functions go stale silently.
