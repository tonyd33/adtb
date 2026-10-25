# Tutorial D

kind: language
home: https://www.dcs.warwick.ac.uk/~hugh/TTM/

## Constraints

- Database constraints are declared by an explicit `CONSTRAINT` statement over any relational expression, e.g. `CONSTRAINT DBC1 IS_EMPTY ( S WHERE STATUS < 1 OR STATUS > 100 )`. [S 2026-10-25] https://www.dcs.warwick.ac.uk/~hugh/M359/Relational-Database-Constraints.pdf
- Every constraint can be written as `IS_EMPTY(r)` for some relational expression r. [S 2026-10-25] https://www.brcommunity.com/articles.php?id=b062
- The Third Manifesto requires constraints checked at statement boundaries, with multiple assignment making compound updates atomic, rather than deferred checking. [V 2026-10-25] (DTATRM ch. 2 and ch. 6 p. 194) https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf
- Type constraints in possreps: `TYPE POSINT POSSREP { I INTEGER CONSTRAINT I > 0 }`. [U] https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf


## Writes

- INSERT, DELETE and UPDATE are shorthands for relational assignment, e.g. a DELETE is `S := S WHERE NOT ( CITY = 'Athens' ) ;`. [V 2026-10-25] https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf ch. 2
- Statements are comma-joined into one multiple assignment ending at `;`; constraint checking is "done at semicolons". [V 2026-10-25] https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf ch. 2, ch. 4 p. 88
