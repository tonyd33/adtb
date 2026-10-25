---
title: Which code generator compiles query pipelines
pick: one
raised-by: [exec/model/compiled, exec/model/adaptive]
options:
  llvm:
    title: LLVM through Rust bindings
    links:
      - {rel: prior-art, to: prior/exec-hyper}
  cranelift:
    title: Cranelift, written in Rust
    links:
      - {rel: prior-art, to: prior/exec-umbra}
  direct:
    title: An own single-pass emitter from the engine's IR
    links:
      - {rel: prior-art, to: prior/exec-umbra}
---
The engine is a Rust crate. Measured on Umbra's queries by Engelke and
Schwarz (CGO 2024).

## exec/backend/llvm

Best generated code; TPC-H Q1 59 ms to compile optimized in HyPer.
Crates such as `inkwell` (unverified as maintained) bind the C++
library, a large native build dependency.

## exec/backend/cranelift

Pure Rust, used in production by Wasmtime; code about 14% slower than
LLVM per its README. On query code it reached unoptimized-LLVM speed
and compiled only 20-35% faster than LLVM; its JIT wrapper was
immature (CGO 2024).

## exec/backend/direct

Umbra's single-pass back-end compiles 16x faster than Cranelift at
similar execution speed, for x86-64 and AArch64; the engine owns an
assembler per target.
