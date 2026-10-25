# libSQL

kind: system
home: https://github.com/tursodatabase/libsql

## Wasm UDFs

- CREATE FUNCTION / DROP FUNCTION register WebAssembly functions; sugar over rows in libsql_wasm_func_table(name, body); build flag --enable-wasm-runtime; Wasmtime or WasmEdge; experimental and opt-in. [V 2026-10-25] https://github.com/tursodatabase/libsql/blob/main/libsql-sqlite3/doc/libsql_extensions.md
