# Prisma

kind: system
home: https://www.prisma.io

## Migrations

- Renaming a field in `schema.prisma` makes `prisma migrate dev` emit create-new-column plus drop-old-column, losing the data; Prisma cannot tell rename from drop and add. [S 2026-10-25] https://github.com/prisma/prisma/discussions/8557
- `prisma migrate dev --create-only` creates a draft migration to edit (e.g. into `ALTER TABLE ... RENAME COLUMN`) before applying. [S 2026-10-25] https://www.prisma.io/docs/orm/prisma-migrate/workflows/customizing-migrations
