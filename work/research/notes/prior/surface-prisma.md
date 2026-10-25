---
title: Prisma schema and Migrate
---
One declarative `schema.prisma`; `prisma migrate dev` diffs it and
emits SQL. A renamed field becomes drop plus create, losing the column;
the user runs `--create-only` and edits the SQL into a `RENAME COLUMN`.
Prisma Client is generated from the schema, not from queries.
https://www.prisma.io/docs/orm/prisma-migrate/workflows/customizing-migrations
https://github.com/prisma/prisma/discussions/8557
