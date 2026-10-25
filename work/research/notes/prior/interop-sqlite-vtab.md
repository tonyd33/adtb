---
title: SQLite virtual tables
---
A virtual table is a module of callbacks; xBestIndex negotiates which constraints and orderings the module serves and at what cost. No triggers, indexes or added constraints on it. FTS5 and R-Tree are virtual tables storing data in shadow tables. VACUUM INTO writes a compacted consistent snapshot as a new file.
https://www.sqlite.org/vtab.html
https://www.sqlite.org/lang_vacuum.html
