---
created-at: 2026-10-27
status: open
kind: bug
priority: 877
summary: "no test pins which form's parse errors the shell reports for a --schema file that parses as neither a schema nor a migration"
tags: [quality, test]
---

# Which parse error a malformed --schema file reports is unpinned

## Report

`first_migration` at `crates/adtb-shell/src/schema.rs:163-167` picks the schema form's or the migration form's parse errors when the file parses as neither. Its only test asserts the message lacks "schema operator", so neither the chosen form nor the reported line and column is pinned.

Fix: assert the exact rendered message, with line and column, for one schema-like and one migration-like near miss.
