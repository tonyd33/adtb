---
title: Whether schema changes are transactional
pick: one
raised-by: [evolution/smo]
options:
  transactional-ddl:
    title: Operators and data writes commit or roll back together
  autocommit-ddl:
    title: Each schema operator commits on its own
---
A split moves rows between constructor heaps and rewrites union
indexes; a scripted migration writes data around operators.

## engine/transactional-ddl

Postgres: DDL inside a transaction, invisible until commit, undone by
rollback (https://wiki.postgresql.org/wiki/Transactional_DDL_in_PostgreSQL:_A_Competitive_Analysis).
SQLite likewise.

```
-- 0014: one transaction; a failure leaves the schema at 13
split constructor Identity.Social into Google { gid: Text } | Facebook { fid: Text }
  by match
    | Social { provider: Google, sub }   -> Google { gid: sub }
    | Social { provider: Facebook, sub } -> Facebook { fid: sub }
```

## engine/autocommit-ddl

MySQL: DDL implicitly commits the open transaction
(https://dev.mysql.com/doc/refman/8.0/en/implicit-commit.html). A
failed script leaves the schema between versions.
