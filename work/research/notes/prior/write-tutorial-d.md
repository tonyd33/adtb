---
title: Tutorial D relational assignment and statement-boundary checking
---
INSERT, DELETE and UPDATE are shorthands for relational assignment
(`S := S WHERE NOT (CITY = 'Athens')`). RM Prescription 21 adds
multiple assignment `A1, A2, ..., An ;` as one operation. Database
constraints are checked at statement boundaries; deferred checking is
prohibited as unnecessary given multiple assignment. Date and Darwen,
Databases, Types and the Relational Model, 3rd ed., ch. 2, ch. 4 p. 88,
ch. 6 p. 194. https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf
