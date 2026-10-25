# PRISM, PRISM++ and BiDEL

kind: paper
home: https://www.vldb.org/pvldb/vol4/p117-curino.pdf

## Integrity constraint evolution (PRISM++)

- Curino, Moon, Deutsch, Zaniolo, "Update Rewriting and Integrity Constraint Maintenance in a Schema Evolution Support System: PRISM++", PVLDB 4(2) 2010. [V 2026-10-25] https://www.vldb.org/pvldb/vol4/p117-curino.pdf
- Constraints supported: primary keys, foreign keys, simple value constraints. ICMO syntax: `ALTER TABLE R ADD PRIMARY KEY pk1(a, b) <policy>`, `ADD FOREIGN KEY fk1(c, d) REFERENCES T (a, b) <policy>`, `ADD VALUE CONSTRAINT vc1 AS R.e = "0" <policy>`, plus drops (six ICMOs). [V 2026-10-25] https://www.vldb.org/pvldb/vol4/p117-curino.pdf
- Policies for adding a constraint: CHECK (roll back the ICMO if the data violates it) and ENFORCE (delete violating tuples, a canonical repair). For the inverse of a drop, used only in rewriting old queries: IGNORE, CHECK, ENFORCE (ENFORCE adds WHERE conditions so no violating tuple is returned). [V 2026-10-25] https://www.vldb.org/pvldb/vol4/p117-curino.pdf
- ICMOs are by definition not information preserving; PRISM++ suggests the sanitizing ICMOs needed before each SMO and reports potential data loss. [V 2026-10-25] https://www.vldb.org/pvldb/vol4/p117-curino.pdf


## PRISM

- SMOs incl. `PARTITION TABLE R INTO S WITH cond, T`, `MERGE TABLE` (not information preserving). [S 2026-10-25] https://www.vldb.org/pvldb/vol1/1453939.pdf
- PARTITION has an exact chase inverse. [S 2026-10-25] https://arxiv.org/pdf/2211.13810
- PRISM++ rewrites legacy queries and updates; ICMOs evolve integrity constraints. [S 2026-10-25] https://www.vldb.org/pvldb/vol4/p117-curino.pdf

## InVerDa / BiDEL (Herrmann et al., SIGMOD 2017)

- `SPLIT TABLE T INTO R WITH cR, S WITH cS`; overlapping rows become twins, uncovered rows go to auxiliary T'. MERGE inverse uses auxiliary tables. Each version is views plus triggers over shared tables; data written in any version readable in all; old-version writes supported. [V 2026-10-25] https://ar5iv.labs.arxiv.org/html/1608.05564
- Survey: Herrmann et al., "Multi-schema-version data management", VLDB J. [S 2026-10-25] https://dl.acm.org/doi/10.1007/s00778-018-0508-7
