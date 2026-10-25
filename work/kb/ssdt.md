# SQL Server Data Tools

kind: system
home: https://learn.microsoft.com/sql/ssdt

## Migrations

- Renames done through SSDT refactoring are recorded in a `.refactorlog`; deployment then emits `sp_rename` instead of drop and create. [S 2026-10-25] https://learn.microsoft.com/sql/ssdt/how-to-use-rename-and-refactoring-to-make-changes-to-your-database-objects
- Deleting the refactor log makes earlier renames deploy as drop and create, losing data, on databases that never received them. [S 2026-10-25] https://learn.microsoft.com/sql/ssdt/how-to-use-rename-and-refactoring-to-make-changes-to-your-database-objects
