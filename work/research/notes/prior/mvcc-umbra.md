---
title: Memory-optimized MVCC for disk-based systems (Freitag et al. VLDB 2022)
---
Umbra: pages hold only the newest version; version chains live only in
memory, reached through a per-page mapping table attached to the buffer
frame; nothing about versions is logged or persisted; recovery starts
with no chains. Writers too large for memory run as exclusive bulk
operations with virtual versions marked by a per-page epoch.
https://www.vldb.org/pvldb/vol15/p2797-freitag.pdf
