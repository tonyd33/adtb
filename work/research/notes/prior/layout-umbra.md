---
title: Umbra variable-size pages
---
Neumann and Freitag, CIDR 2020. Page size classes from 64 KiB doubling
up to the pool size, each in its own virtual range; pointer swizzling.
Relations are B+-trees on synthetic 8-byte tids filled without splits;
leaves use PAX (fixed attributes columnar at the front, variable data at
the back). 16-byte string headers: 4-byte length, then 12 inline bytes
or a 4-byte prefix plus offset. Disk space reused only for same-size
pages, for ARIES safety.
https://www.cidrdb.org/cidr2020/papers/p29-neumann-cidr20.pdf
