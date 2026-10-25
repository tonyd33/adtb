---
title: Oracle SQL plan baselines
---
A baseline is the set of accepted plans for a statement. New plans go
to plan history unaccepted and are accepted after verification shows
they perform at least as well. Fixed plans are preferred. When no
baseline plan is reproducible the optimizer falls back to a new plan.
https://docs.oracle.com/en/database/oracle/oracle-database/23/tgsql/overview-of-sql-plan-management.html
