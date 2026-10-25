---
title: Compiled versus vectorized, measured in one system
---
Kersten et al., VLDB 2018: similar OLAP speed; compiled wins on
computation (Q1 +74%), vectorized on hash probing (Q9 +32%).
Vectorized wins at compile time, profiling and adaptivity; compiled at
OLTP stored procedures. https://www.vldb.org/pvldb/vol11/p2209-kersten.pdf
