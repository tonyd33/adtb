# Peyton Jones, Reid, Hoare, Marlow, Henderson, A semantics for imprecise exceptions (PLDI 1999)

kind: paper
home: https://www.microsoft.com/en-us/research/wp-content/uploads/1999/05/except.pdf

## Exceptions in a pure language

- Argues imprecision is essential if a pure language keeps its algebra of transformations: fixing which exception is raised bans reorderings. [V 2026-10-25] abstract, sec. 1
- An exceptional value denotes a set of exceptions; e.g. (1/0) + (error "Urk") denotes the union of both operands' sets, so + stays commutative. [V 2026-10-25] sec. 3-4
- getException is in IO: choosing which member of the set is reported is a nondeterministic choice made when the computation is performed, not in the pure semantics. [V 2026-10-25] sec. 4
- Cites IEEE NaNs and infinities as exceptional values that propagate through operations. [V 2026-10-25] sec. 3
