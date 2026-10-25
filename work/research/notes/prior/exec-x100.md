---
title: MonetDB/X100 vectorized execution
---
Boncz, Zukowski, Nes, CIDR 2005: pull of 1024-value vectors through
type-specialised primitives, each taking an optional selection vector.
TPC-H Q1 SF1 0.50 s against MySQL 26.6 s; MySQL spent about 49 cycles
per addition. https://www.cidrdb.org/cidr2005/papers/P19.pdf
