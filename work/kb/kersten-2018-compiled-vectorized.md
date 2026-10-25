# Kersten et al., "Everything You Always Wanted to Know About Compiled and Vectorized Queries But Were Afraid to Ask" (VLDB 2018)

kind: paper
home: https://www.vldb.org/pvldb/vol11/p2209-kersten.pdf

## Findings

- Typer (data-centric compiled) and Tectorwise (vectorized) built in one test system with the same algorithms and data structures. Vectorization hides cache miss latency better; compilation needs fewer instructions, benefiting cache-resident work. [V 2026-10-25] abstract
- Typer faster by 74% on Q1 (computation bound); Tectorwise faster by 32% on Q9 and 4% on Q3 (hash probing). [V 2026-10-25] sec. 4.1
- Vector size 1,000 default (also VectorWise's); <64 and >64K degrade; Q3 15% faster at 64K. Vector size 1 makes Tectorwise a Volcano interpreter. [V 2026-10-25] sec. 4.4, Fig. 5
- LLVM compile time can be super-linear in code size; a table with thousands of columns generates much code. HyPer replaces LLVM register allocation and runs the first morsels in an LLVM IR interpreter. Spark falls back to interpretation when a pipeline exceeds 8 KB of Java bytecode. [V 2026-10-25] sec. 8.2
- Summary: OLAP performance similar. Compilation wins at OLTP (stored procedures) and language integration; vectorization wins at compile time (precompiled primitives), profiling (time attributable to primitives) and adaptivity (primitives swappable mid-flight). [V 2026-10-25] sec. 10
