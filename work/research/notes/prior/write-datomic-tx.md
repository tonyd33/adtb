---
title: Datomic transactions as unordered data
---
A transaction is a set of assertions and retractions with no order,
resolved against db-before; tempids unify with existing entities
through `:db.unique/identity` (upsert) and come back in `:tempids`.
Entity predicates see db-after. https://docs.datomic.com/transactions/model.html
