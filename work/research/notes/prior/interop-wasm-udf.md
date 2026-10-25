---
title: WebAssembly UDFs
---
SingleStore CREATE FUNCTION ... AS WASM with a basic or canonical (WIT) ABI and bounded linear memory, also TVFs and aggregates. libSQL CREATE FUNCTION stores module text in a catalog table (experimental). Extism wraps a runtime with byte in/out and limits. Wasmtime fuel is deterministic metering; epochs are cheaper and nondeterministic. WIT has variant, enum, option, result and record types.
https://docs.singlestore.com/cloud/reference/code-engine-powered-by-wasm/create-wasm-udfs/
https://github.com/tursodatabase/libsql/blob/main/libsql-sqlite3/doc/libsql_extensions.md
https://github.com/extism/extism
https://docs.wasmtime.dev/api/wasmtime/struct.Config.html
https://component-model.bytecodealliance.org/design/wit.html
