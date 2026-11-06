---
created-at: 2026-11-05
status: open
kind: feature
priority: 560
summary: "the archive copies only complete segments, so a restore reaches at most the end of the last complete segment; no forced switch bounds that"
---

# Feature: A forced log segment switch for the archive

## Request

## Proposal

## Open questions

## 2026-11-05

From S6a. The archive (storage/archive.rs) publishes only complete, forced segments; the tail is never archived, so on a quiet database the archive can lag by up to a segment indefinitely. PostgreSQL bounds this with archive_timeout (work/kb/postgres.md). Direction: an open setting that forces a segment switch after a period with unarchived commits, or archive the tail under a partial name that S7 can read as the last segment.
