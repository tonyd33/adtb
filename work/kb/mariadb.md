# MariaDB

kind: system
home: https://mariadb.com/docs/server

## Temporal

- `WITH SYSTEM VERSIONING`; `FOR SYSTEM_TIME AS OF TIMESTAMP`, `BETWEEN`, `ALL`; a history row per UPDATE naming a versioned column even if nothing changes. [V 2026-10-25] https://mariadb.com/docs/server/reference/sql-structure/temporal-tables/system-versioned-tables
- InnoDB transaction-precise history: BIGINT UNSIGNED row start/end hold transaction ids, `AS OF TRANSACTION n`, mapped in `mysql.transaction_registry`. [V 2026-10-25] same
- `PARTITION BY SYSTEM_TIME INTERVAL 1 WEEK | LIMIT n` separates current from history; `DELETE HISTORY FROM t BEFORE SYSTEM_TIME ts` prunes; `system_versioning_alter_history` gates ALTER on versioned tables. [V 2026-10-25] same
