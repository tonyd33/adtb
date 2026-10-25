# Graefe, "Volcano: An Extensible and Parallel Query Evaluation System" (IEEE TKDE 6(1), 1994)

kind: paper
home: https://dl.acm.org/doi/10.1109/69.273032

## Execution

- All operators are written for a single process and parallelized by an exchange operator. [S 2026-10-25] search snippet of the paper
- Operators are iterators with open, next, close; demand-driven, one record per next. [U]
