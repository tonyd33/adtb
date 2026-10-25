---
title: SSDT refactor log
---
SQL Server Data Tools diffs a declarative project against a database.
Renames done through the refactoring UI are recorded in a
`.refactorlog`, so deployment emits `sp_rename` instead of drop and
create. Deleting the log turns past renames back into drop and create.
https://learn.microsoft.com/sql/ssdt/how-to-use-rename-and-refactoring-to-make-changes-to-your-database-objects
