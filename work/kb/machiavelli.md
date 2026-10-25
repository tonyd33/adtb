# Machiavelli

kind: language
home: https://doi.org/10.1145/67544.66931

## Types

- Ohori, Buneman, Breazu-Tannen, "Database programming in Machiavelli, a polymorphic language with static type inference", SIGMOD 1989. [S 2026-10-25] https://dl.acm.org/doi/abs/10.1145/227604.227609
- Structural records with polymorphic field selection, variants, and sets; `ref` types give object identity and cyclic structures. [U]
- `ref(τ)` ranges over all types; database objects are implemented as reference types, with views as sets of structures with identity. The SML/NJ prototype implements everything except reference types and cyclic data. [S 2026-10-25] https://homepages.inf.ed.ac.uk/opb/papers/TODS1996.pdf
- Generalizes join and projection to arbitrary complex objects; statically typed programs over heterogeneous collections. [S 2026-10-25] https://homepages.inf.ed.ac.uk/opb/papers/TODS1996.pdf
- SIGMOD 1989 paper is pp. 46-57, DOI 10.1145/67544.66931. [S 2026-10-25] https://dl.acm.org/doi/10.1145/67544.66931

## Design

- Ohori, Buneman, Breazu-Tannen, SIGMOD 1989: heterogeneous collections via kinded type variables, not subtyping. [V 2026-10-25] ; variant `case` syntax [U]
