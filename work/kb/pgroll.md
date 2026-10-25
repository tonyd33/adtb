# pgroll

kind: system
home: https://github.com/xataio/pgroll

## Versioned schemas

- Creates virtual schemas of views over physical tables; during a migration old and new versions are both accessible, chosen by `search_path`. [V 2026-10-25] https://github.com/xataio/pgroll
- Breaking changes create a new physical column backfilled from the old, with triggers syncing writes; completing removes the old version. [V 2026-10-25] https://github.com/xataio/pgroll
