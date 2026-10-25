# Cranelift

kind: system
home: https://github.com/bytecodealliance/wasmtime/tree/main/cranelift

## Status

- README: production-ready, used in production within Wasmtime; compile speed about an order of magnitude faster than WAVM (LLVM) in a 2020 paper; generated code about 14% slower than WAVM and 2% slower than V8 TurboFan. A JIT demo exists. [V 2026-10-25] https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/README.md
- Written in Rust. [V 2026-10-25] Engelke, Schwarz CGO 2024 sec. VI
