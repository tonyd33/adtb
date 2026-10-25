---
title: Which host-language code may define functions
pick: any
options:
  wasm:
    title: Sandboxed WebAssembly modules stored in the catalog
    links:
      - {rel: prior-art, to: prior/interop-wasm-udf}
  host-fn:
    title: Rust functions registered by the embedding program
  native-plugin:
    title: Shared libraries loaded into the engine process
    assessment: {grade: costly, why: a fault in the plugin is a fault in the engine and its file}
    links:
      - {rel: prior-art, to: prior/interop-pg-extensions}
      - {rel: prior-art, to: prior/interop-duckdb-io}
---
Functions in the query language are already schema objects
(`lang/defs-home/schema`, `lang/abstraction/higher-order`). This asks
what else can define one. Taking none leaves query-language functions
only.

## interop/udf/wasm

```
fn geohash (lat: F64, lon: F64, prec: I32) -> Text
  = wasm "geo.wasm" export "geohash"
```

The module bytes are a catalog row, dumped and replicated with the
schema. SingleStore `CREATE FUNCTION ... AS WASM` with a WIT-typed ABI
and bounded linear memory
(https://docs.singlestore.com/cloud/reference/code-engine-powered-by-wasm/create-wasm-udfs/);
libSQL keeps module text in `libsql_wasm_func_table`
(https://github.com/tursodatabase/libsql/blob/main/libsql-sqlite3/doc/libsql_extensions.md).
Wasmtime meters with fuel or epochs
(https://docs.wasmtime.dev/api/wasmtime/struct.Config.html).

## interop/udf/host-fn

```rust
db.register_fn("slugify", |t: &str| -> String { slug::slugify(t) })?;
```

Embedded mode only (`surface/wire/embedded`); the schema declares the
signature, the program supplies the body at open. SQLite
`sqlite3_create_function` (unverified). Another process opening the
file without the function cannot evaluate what uses it.

## interop/udf/native-plugin

Postgres C functions in a shared library; DuckDB loadable extensions
pinned to a release or to a versioned C API
(https://duckdb.org/docs/2.0/extensions/overview.html).
