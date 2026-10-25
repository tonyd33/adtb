---
title: Online, asynchronous schema change in F1
---
Rae et al., PVLDB 6(11), 2013. A change is split into steps through
intermediate states (delete-only, write-only) so that servers at most
one schema version apart never corrupt data; no global lock.
https://research.google.com/pubs/pub41376.html
