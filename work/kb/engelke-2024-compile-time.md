# Engelke, Schwarz, "Compile-Time Analysis of Compiler Frameworks for Query Compilation" (CGO 2024)

kind: paper
home: https://home.cit.tum.de/~engelke/pubs/2403-cgo.pdf

## Findings

- Umbra back-ends: interpreter, C compiler, LLVM, Cranelift, and a single-pass direct emitter to x86-64. [V 2026-10-25] sec. I
- LLVM gives the best execution performance and can compile much faster when tuned for compile time. Cranelift reaches run-time performance similar to unoptimized LLVM but compiles only 20-35% faster; Umbra's single-pass back-end compiles 16x faster than Cranelift at similar execution performance. [V 2026-10-25] abstract, sec. IX
- Cranelift's JIT wrapper was "not quite mature" at the time. [V 2026-10-25] sec. VI
