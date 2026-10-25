---
title: How SQLite is tested
---
Four independent harnesses (TCL, TH3 at 100% MC/DC, SQL Logic Test
against other engines, dbsqlfuzz), OOM, I/O error and crash tests,
the whole suite rerun with optimizations disabled, ALWAYS()/NEVER()
defensive macros, mutation testing. Test code is about 590 times the
library.
https://www.sqlite.org/testing.html
