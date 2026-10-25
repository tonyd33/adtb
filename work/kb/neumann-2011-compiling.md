# Neumann, "Efficiently Compiling Efficient Query Plans for Modern Hardware" (VLDB 2011)

kind: paper
home: https://www.vldb.org/pvldb/vol4/p539-neumann.pdf

## Execution model

- Data-centric: pipeline fragments push tuples from one pipeline breaker to the next, keeping attributes in CPU registers; operator boundaries are blurred. An operator is a pipeline breaker for an input side if it takes an incoming tuple out of registers; full pipeline breaker if it materializes all input. [V 2026-10-25] PVLDB 4(9) p539, sec. 3
- Code generation interface per operator: `produce()` and `consume(attributes, source)`; produce asks for result tuples, which are pushed to the parent's consume. [V 2026-10-25] sec. 3.2
- Mixed execution: complex logic (index structures, spilling) is precompiled C++ ("cogwheels"); only the LLVM "chain" connecting them is generated per query. [V 2026-10-25] sec. 4
- Generating C++ and compiling at runtime took multiple seconds for a complex query; LLVM usually needs a few milliseconds. [V 2026-10-25] sec. 4

## Numbers

- TPC-C: HyPer+C++ 161,794 tps with 16.53 s total compile time; HyPer+LLVM 169,491 tps with 0.81 s. [V 2026-10-25] Table 1
- TPC-CH Q3 warm ms: HyPer C++ 141 (compile 1976), HyPer LLVM 80 (compile 30), VectorWise 257, MonetDB 112, DB X 16410. LLVM backend frequently 2-4x faster than the other systems. [V 2026-10-25] Table 2, sec. 6
