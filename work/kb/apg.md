# Algebraic Property Graphs

kind: paper
home: https://arxiv.org/abs/1909.04881

## Types and identity

- Schema S = (L, sigma): finite label set L, sigma maps each label to a type `t ::= 0 | 1 | t1+t2 | t1*t2 | Prim p | Lbl l`. [V 2026-10-25] https://arxiv.org/html/1909.04881 sec. 3
- An APG assigns each label a set of elements E(l) and a value function E(l) -> V(sigma(l)); V(Lbl l) = E(l), so a label type is a reference to an element. [V 2026-10-25] https://arxiv.org/html/1909.04881 sec. 3
- Elements carry identity: each element has a unique identifier and exactly one label. [V 2026-10-25] https://arxiv.org/html/1909.04881 sec. 3.1
- Cyclic references through labels are allowed (sigma(A) = A + 1 gives streams); the authors say recursive types are not possible without meta-edges. [V 2026-10-25] https://arxiv.org/html/1909.04881 sec. 4.3, 6.2

## Claims

- Types are ADTs and join-union E/R diagrams. [V 2026-10-25] https://arxiv.org/abs/1909.04881
- Constraints listed as future work. [S 2026-10-25]
