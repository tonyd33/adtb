---
title: SQL:2011 temporal
---
Kulkarni and Michels, SIGMOD Record 2012. System-versioned tables (system time, set only by the system) and application-time period tables (valid time); FOR PORTION OF splits rows; WITHOUT OVERLAPS keys; temporal FKs need the child period inside the union of parent periods. SQL Server keeps a separate history table; MariaDB can partition history and DELETE HISTORY. https://sigmodrecord.org/publications/sigmodRecord/1209/pdfs/07.industry.kulkarni.pdf
