# SQL standard

kind: language
home: https://jakewheat.github.io/sql-overview/

## Structured types

- SQL:2003 grammar: `TREAT(x AS T)`, `x IS [NOT] OF ([ONLY] T)`, `ONLY(table)`. [V 2026-10-25] https://jakewheat.github.io/sql-overview/sql-2003-foundation-grammar.html
- SQLSTATE 0D000 "invalid target type specification" exists; that TREAT raises it [U]. [V 2026-10-25] https://en.wikipedia.org/wiki/SQLSTATE
- SQL:1999 subtable row-id type is a subtype of each supertable's; `REFERENCES ARE CHECKED` implies deferrable initially immediate. [S 2026-10-25] https://docs.cratedb.com/projects/sql-99/en/latest/chapters/18.html

## Temporal (SQL:2011)

- Kulkarni and Michels, "Temporal features in SQL:2011", SIGMOD Record 41(3) 2012. Valid time is application-time period tables; transaction time is system-versioned tables (period named SYSTEM_TIME); at most one application-time period and one system-time period per table; closed-open periods. [V 2026-10-25] https://sigmodrecord.org/publications/sigmodRecord/1209/pdfs/07.industry.kulkarni.pdf
- `UPDATE/DELETE ... FOR PORTION OF p FROM a TO b` splits overlapping rows so only the portion changes; `PRIMARY KEY (k, p WITHOUT OVERLAPS)`; temporal FKs require the child period contained in the union of matching parent periods. [V 2026-10-25] same
- System-time columns are set only by the system from the transaction timestamp, so history cannot be tampered with. [V 2026-10-25] same

## Document structure and conformance

- ISO/IEC 9075 in parts: 1 Framework, 2 Foundation, 3 CLI, 4 PSM, 9 MED, 10 OLB, 11 Schemata, 13 JRT, 14 XML, 15 MDA, 16 PGQ. Each version replaces the previous; latest 2023. [V 2026-10-25] https://www.postgresql.org/docs/current/features.html
- SQL-92 had Entry/Intermediate/Full levels; since SQL:1999 individual features, a Core subset mandatory, rest optional. [V 2026-10-25] same
