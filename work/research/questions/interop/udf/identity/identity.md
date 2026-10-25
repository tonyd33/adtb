---
title: Changing a function's code changes indexes and constraints built on it
pick: any
raised-by: [interop/udf/wasm, interop/udf/host-fn, interop/udf/native-plugin]
options:
  content-hash:
    title: A function's identity includes a hash of its code; replacing it is a schema operator that rebuilds and rechecks dependents
    links:
      - {rel: gains, to: evolution/breaks/recompile}
  live:
    title: Live with dependents silently computed by old and new code
    excludes: [interop/udf/identity/content-hash]
---
An index on `geohash(lat, lon)` built with version 1 of the module
holds wrong keys once version 2 rounds differently.

## interop/udf/identity/content-hash

```
redefine fn geohash = wasm "geo-v2.wasm" export "geohash"
-- rebuilds index events_geo, rechecks constraint near_store
```

For `host-fn` the program supplies a version string at registration;
opening with a different one is a schema change.

## interop/udf/identity/live

Postgres lets `CREATE OR REPLACE FUNCTION` change an immutable
function under an index (unverified).
