# Pierce and Turner, Local Type Inference (TOPLAS 2000)

kind: paper
home: https://www.cis.upenn.edu/~bcpierce/papers/lti-toplas.pdf

## Inference

- TOPLAS 22(1), 2000, revised from POPL 1998. Partial inference for subtyping plus impredicative polymorphism; missing annotations recovered only from adjacent syntax-tree nodes, with no unification variables. [V 2026-10-25] abstract
- Two techniques: local synthesis of type arguments (subtyping constraints solved to make the result type minimal) and bidirectional propagation of expected types into unannotated lambdas and local bindings. [V 2026-10-25] sec. 1.2
- Design argued from a survey of ML code: annotations on top-level function definitions are acceptable since they document; local bindings should not need them. [V 2026-10-25] sec. 1.1
- A Bot type is added so least upper bounds and greatest lower bounds always exist; joins may fail to exist in full F-sub. [V 2026-10-25] sec. 2.1, sec. on bounded quantification
