# Dolan and Mycroft, MLsub (POPL 2017)

kind: paper
home: https://www.repository.cam.ac.uk/bitstream/handle/1810/261583/Dolan_and_Mycrof-2017-POPL-AM.pdf

## Type inference with subtyping

- "Polymorphism, Subtyping, and Type Inference in MLsub", POPL 2017: type system combining subtyping and ML-style parametric polymorphism with decidable inference and compact principal types; types a strict superset of core ML programs. [V 2026-10-25] author's accepted manuscript, abstract
- Types form a distributive lattice with join and meet; functions contravariant in domain, covariant in range; records get width and depth subtyping, and the upper bound of two record types has the intersection of their fields. [V 2026-10-25] sec. 1.1, 2
- Polar types: joins only in positive (output) positions, meets only in negative (input) positions; bottom only on outputs, top only on inputs; every typeable expression has a principal polar typing scheme. [V 2026-10-25] sec. 3.2
- Inference by biunification, a variant of unification over constraints of the form positive <= negative. [V 2026-10-25] sec. 4
- Type simplification via automata: two automata represent equivalent types iff they accept equal languages. [V 2026-10-25] sec. 1, 5.3
- Subsumption between type schemes (needed to check an inferred type against a user annotation or signature) is decidable, proved in Dolan's thesis ch. 8. [V 2026-10-25] sec. 1.1
- Invariant mutable references are encoded with one contravariant and one covariant parameter. [V 2026-10-25] sec. 1.1
- Calculus covers booleans, records, functions and let; no sums or polymorphic variants in the paper's core (claimed in thesis/prototype only). [V 2026-10-25] sec. 1.1
