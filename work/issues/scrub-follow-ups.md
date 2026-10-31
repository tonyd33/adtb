---
created-at: 2026-10-30
status: open
kind: bug
priority: 470
summary: "scrub: an allocated page nothing reaches marks the database; a due open reads the whole retained log; close writes the cursor; a transient read error refuses a page; a busy daemon with main parked hangs the simulator; a torn read has no regression test"
---

# Scrub follow ups

## Report

Also: the mark is durable before its finding row, so a crash between them leaves a relation quarantined with no row saying why; a panic in the log read at the start of a pass is not caught; a free-page bitmap that is unreadable or of the wrong kind.
