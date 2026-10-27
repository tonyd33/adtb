---
created-at: 2026-10-27
status: open
kind: bug
priority: 859
summary: "the reader rules build drop operators as migration text and parse it back, and Debug-print the checker's diagnostics into Error::Internal"
tags: [quality, engine]
---

# Releasing a held drop formats migration text and reparses it, with errors as Debug strings

## Report

`without` at `crates/adtb/src/catalog/readers.rs:658-673` builds each pending drop as migration text with `format!` and parses it back to apply it. `checked` and `without` (`:677-683`) map the checker's diagnostics into `Error::Internal(format!("{e:?}"))`. A name that needs quoting breaks the round trip, and a failure reads as nested Debug text.

Fix: build the drop operators directly (the `MigrationFile` operator values), and keep the diagnostics typed in the error.
