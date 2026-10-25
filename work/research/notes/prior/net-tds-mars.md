---
title: SQL Server MARS
---
Several pending requests and open result sets on one TDS connection,
interleaved only at yield points (SELECT, FETCH, RECEIVE), not run in
parallel. A transaction begun in a batch must end in it.
https://learn.microsoft.com/en-us/sql/relational-databases/native-client/features/using-multiple-active-result-sets-mars
