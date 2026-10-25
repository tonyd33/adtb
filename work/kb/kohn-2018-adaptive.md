# Kohn, Leis, Neumann, "Adaptive Execution of Compiled Queries" (ICDE 2018)

kind: paper
home: https://db.in.tum.de/~leis/papers/adaptiveexecution.pdf

## Findings

- A pgAdmin catalog query executes in under 1 ms in HyPer but LLVM with optimizations takes 54 ms to compile it. TPC-H Q1 compiles in 59 ms; largest TPC-H query 146 ms, largest TPC-DS 911 ms. [V 2026-10-25] sec. I-II
- Three modes: bytecode interpretation (lowest latency), unoptimized machine code, optimized machine code. Every query starts in a bytecode interpreter of LLVM IR; per pipeline, progress is extrapolated to choose whether to compile; switching loses no work because morsels are the unit. [V 2026-10-25] sec. III-IV
- Caching machine code after stripping constants would help repeat executions but not the first. [V 2026-10-25] sec. I
