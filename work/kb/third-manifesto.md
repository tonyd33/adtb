# The Third Manifesto

kind: paper
home: https://www.dcs.warwick.ac.uk/~hugh/TTM/

## Model

- TUPLE and RELATION are type generators (RM Prescriptions 6, 7); a tuple or relation type is determined by its heading. [S 2026-10-25] https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf
- Relation-valued attributes are part of the model. [S 2026-10-25] https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf
- OO Proscription 2: no object IDs. RM Proscription 4: no nulls. [S 2026-10-25] https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf
- Every relvar has at least one candidate key (RM Prescription 15). [U]

## Assignment and constraints

- RM Prescription 21: D supports `:=` for every type and a multiple assignment `A1, A2, ..., An ;` performed as a single operation; repeated targets are folded with `WITH Xp AS Vq : Xq`, then all sources evaluated, then assigned. [V 2026-10-25] Date and Darwen, Databases, Types and the Relational Model, 3rd ed., ch. 4, p. 88, https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf
- DELETE is "logically equivalent to, and indeed shorthand for" a relational assignment `S := S WHERE NOT (...)`; INSERT/DELETE/UPDATE are only shorthands. [V 2026-10-25] DTATRM ch. 2 and ch. 4, same PDF
- Database constraints are checked at statement boundaries ("at semicolons"), statement meaning relational assignment; a failing check undoes the statement. [V 2026-10-25] DTATRM ch. 2, same PDF
- RM Prescription 23 discussion: deferred (commit-time) checking is prohibited, logically unnecessary given multiple assignment; "No user shall ever see a state of affairs in which [any constraint] is not satisfied." [V 2026-10-25] DTATRM ch. 6, p. 194, same PDF
- RM Prescription 15: every relvar has at least one candidate key, defined explicitly or implicitly at relvar definition; its keys cannot all be destroyed short of destroying the relvar. [S 2026-10-25] https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf (via search snippet)

## Inheritance model

- Specialization by constraint: most specific type follows from the value; each value has exactly one most specific type. [S 2026-10-25] https://www.dcs.warwick.ac.uk/~hugh/TTM/DTATRM.pdf ; prescription numbers [U]
- Union types: types with no possrep whose values are exactly their subtypes'. [S 2026-10-25] same
