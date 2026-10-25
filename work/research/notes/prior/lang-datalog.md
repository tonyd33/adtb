---
title: Stratified Datalog
---
Negation and aggregation allowed only on relations in a strictly lower
stratum; a program with a cycle through negation is rejected. Apt,
Blair, Walker, "Towards a theory of declarative knowledge", 1988.
Soufflé and Flix both stratify. Flix also computes fixpoints on
lattices, admitting monotone aggregation inside recursion (Madsen, Yee,
Lhoták, PLDI 2016, https://doi.org/10.1145/2908080.2908096).
