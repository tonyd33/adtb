# Maranget: Compiling Pattern Matching to Good Decision Trees (ML 2008)

kind: paper
home: http://moscova.inria.fr/~maranget/papers/ml05e-maranget.pdf

## Match compilation

- Matching automata are decision trees or backtracking automata. Backtracking automata (Augustsson 1985) guarantee code size linear in the patterns but may test a subterm more than once; decision trees never test a subterm twice but may explode in code size. [V 2026-10-29] §1 of the PDF above
- Decision trees are traditionally implemented as DAGs with maximal sharing. The prototype targets a simplified OCaml first intermediate language whose local functions implement maximal sharing or backtracking. [V 2026-10-29] abstract and §8
- The OCaml match compiler is the optimizing backtracking compiler of Le Fessant and Maranget, "Optimizing pattern matching", ICFP 2001. [V 2026-10-29] §1, §8
