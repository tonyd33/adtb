# SingleStore

kind: system
home: https://docs.singlestore.com/

## Wasm UDFs

- CREATE FUNCTION f AS WASM [ABI BASIC|CANONICAL] [GROW TO n] FROM <source> [WITH WIT FROM ...] [USING EXPORT ...]; canonical ABI takes types from WIT; default 16 MB linear memory; any wasm32-wasi language, Rust and C documented. Also Wasm TVFs and UDAFs. [V 2026-10-25] https://docs.singlestore.com/cloud/reference/code-engine-powered-by-wasm/create-wasm-udfs/
