---
title: Oracle substitutable object tables
---
One table holds a whole type hierarchy: a hidden column for each
attribute added in any subtype, plus a hidden typeid discriminant read by
`SYS_TYPEID`. DB2 typed-table hierarchies use one hierarchy table with a
typecode column (unverified in IBM manuals).
https://docs.oracle.com/en/database/oracle/oracle-database/26/adobj/storage-of-objects.html
