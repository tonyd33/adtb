---
title: SQL Server ledger
---
Rows changed per transaction hashed into Merkle trees, transactions
into blocks chained by hash; the latest digest is exported to WORM
storage and verification recomputes it. Detects, cannot prevent, file
tampering by an administrator.
https://learn.microsoft.com/en-us/sql/relational-databases/security/ledger/ledger-overview
