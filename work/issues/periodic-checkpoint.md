---
created-at: 2026-11-05
status: open
kind: feature
priority: 420
summary: "the engine checkpoints only at open, close and backup, so dirty pages, redo and the retained log grow without bound within one session"
---

# Feature: Checkpoint periodically

## Request

## Proposal

## Open questions

## 2026-11-05

Found by the readin-gate track: only flush_pages cleans the dirty page table and nothing checkpoints between open and close, so redo never advances within a session and recycling (persisted-lsn-map) bounds the log only per session. A periodic checkpointer must keep the recycle floor rule: never past the start of the current session.

The readin-gate track dropped the backup checkpoint pin: while recycling runs only at open, the session floor already keeps every segment a backup copies. A checkpoint that moves the recycle floor within a session must restore a pin at the backup's redo, and the test recycles_while_a_backup_copies_its_pages_keep_the_log_it_copies must then fail with that pin removed.
