# HoTTSQL (Chu, Weitz, Cheung, Suciu, PLDI 2017)

kind: paper
home: https://arxiv.org/abs/1607.04822

## Semantics

- Mechanised denotational semantics of SQL in Coq based on K-relations and homotopy type theory (relations as functions from tuples to univalent types), covering bags, correlated subqueries, aggregation, indexes; under 300 lines of Coq per the arXiv version. [V 2026-10-25] arXiv:1607.04822v2
- Used to prove query rewrite rules, including subquery elimination; a decision procedure for conjunctive queries. [V 2026-10-25] same; 23 rules per the PLDI abstract [S 2026-10-25] https://dl.acm.org/doi/abs/10.1145/3140587.3062348
