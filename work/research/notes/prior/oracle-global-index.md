---
title: Oracle global indexes on partitioned tables
---
Rowids carry the segment's data object number. DROP PARTITION marks
global indexes UNUSABLE unless `UPDATE INDEXES`; since 12c that is
metadata only, leaving orphaned entries that scans skip and
`PMO_DEFERRED_GIDX_MAINT_JOB`, `DBMS_PART.CLEANUP_GIDX` or `ALTER INDEX
... COALESCE CLEANUP` remove.
https://docs.oracle.com/en/database/oracle/oracle-database/19/vldbg/maintenance-partition-can-be-performed.html
