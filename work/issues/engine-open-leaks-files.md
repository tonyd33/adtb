---
created-at: 2026-10-26
status: open
kind: bug
priority: 699
summary: an Engine::open that fails after the pager exists drops the pager without closing its files
---

# Engine open leaks files

## Report

## Report

When `Engine::open` fails after the pager exists (catalog open or a migration refused), the pager is dropped without `Pager::close`, and its files stay open until the I/O pool threads exit. A failed `Pager::open` or `create` already closes its files.
