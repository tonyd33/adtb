---
title: Tutorial D database constraints
links:
  - {rel: prior-art, to: prior/third-manifesto}
---
`CONSTRAINT DBC1 IS_EMPTY (S WHERE STATUS < 1 OR STATUS > 100)`: a
named constraint over any relational expression; every constraint can
be put as `IS_EMPTY(r)`. Checked at statement boundaries, with multiple
assignment for compound updates (unverified). Type constraints live in
possreps. https://www.dcs.warwick.ac.uk/~hugh/M359/Relational-Database-Constraints.pdf
