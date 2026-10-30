---
created-at: 2026-10-30
status: in-progress
branch: server-fixes
kind: bug
priority: 405
summary: "Capability::Maintain is declared and no command requires it, so verify and vacuum have no check over a server"
---

# Maintain capability unchecked

## Report

Found by the verify-online track. Catalog section 9 names `maintain` for these commands. Check every command against its named capability, and test a role lacking it.
