---
title: Oracle materialized view refresh
---
ON COMMIT, ON STATEMENT (inside the DML, rolled back with it) or ON DEMAND; FAST refresh needs MV logs on every base table and COUNT(*) beside aggregates; EXPLAIN_MVIEW reports which methods apply. https://docs.oracle.com/en/database/oracle/oracle-database/19/dwhsg/refreshing-materialized-views.html
