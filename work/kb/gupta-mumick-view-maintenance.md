# Gupta, Mumick, Subrahmanian view maintenance

kind: paper
home: https://doi.org/10.1145/170035.170066

## Citation

- Gupta, Mumick, Subrahmanian, "Maintaining Views Incrementally", SIGMOD 1993: counting algorithm for non-recursive views, DRed (delete and rederive) for recursive ones. [U] https://doi.org/10.1145/170035.170066
- Gupta and Mumick, "Maintenance of Materialized Views: Problems, Techniques, and Applications", IEEE Data Eng. Bull. 18(2), 1995. [U]


## Algorithms (read from scanned text, OCR poor)

- Counting stores, per view tuple, the number of alternative derivations; works for set and duplicate semantics; for nonrecursive views with negation and aggregation. [V 2026-10-25] http://www.cs.columbia.edu/~gravano/Qual/Papers/13%20-%20Maintaining%20Views%20Incrementally.pdf
- DRed (delete and rederive) handles recursive views: first deletes a superset of the tuples to be deleted, then rederives some of them. [V 2026-10-25] same
