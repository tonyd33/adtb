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
