---
title: How an extension is packaged and installed
pick: one
raised-by: [interop/udf/wasm, interop/udf/native-plugin, interop/scalar/opaque, interop/index-am/am-trait, interop/index-am/framework]
options:
  catalog-module:
    title: An extension is a module whose definitions and Wasm code are catalog rows, installed by schema operators
    links:
      - {rel: gains, to: surface/namespace/modules}
      - {rel: gains, to: surface/source/op-log}
  linked:
    title: Extensions are Rust crates compiled into the binary; the database records which it needs
  loadable:
    title: Extensions are shared libraries found and loaded at open
    links:
      - {rel: prior-art, to: prior/interop-duckdb-io}
      - {rel: prior-art, to: prior/interop-pg-extensions}
---
Postgres extensions are a control file, versioned scripts and update
scripts chained by `ALTER EXTENSION UPDATE`
(https://www.postgresql.org/docs/current/extend-extensions.html);
DuckDB installs per machine and loads per session
(https://duckdb.org/docs/2.0/extensions/overview.html).

## interop/ext-pkg/catalog-module

```
-- 0043
install geo 1.2 from "geo-1.2.adtx"
```

Self-contained: the file carries the code, so dumps, replicas and
another process opening the file have it. Only Wasm and
query-language code qualifies.

## interop/ext-pkg/linked

```toml
adtdb = { version = "1", features = ["vector", "fts"] }
```

The file header lists required extensions and versions; native index
kinds and types become possible with no ABI.

## interop/ext-pkg/loadable

`load "libgeo.so"`; C ABI or versioned function table.
