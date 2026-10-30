---
created-at: 2026-10-30
status: open
kind: bug
priority: 400
summary: the machine lock is held by a daemon that inherits its descriptor and is per HOME or XDG_CACHE_HOME; apps/campaign/target has no cap outside --clean
---

# Tool lock gaps

## Report

A daemon started under `check` inherits descriptor 9 and keeps the machine lock after `check` exits. A process with another HOME or XDG_CACHE_HOME takes another lock file, so two full runs overlap. Only serialisation is lost. apps/campaign/target has no size cap outside `workspace-gc --clean`.
