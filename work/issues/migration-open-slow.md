---
created-at: 2026-10-26
status: open
kind: feature
priority: 790
summary: "opening a database replays each migration of its log at about 4 seconds of CPU apiece, so the campaign app's engine tests take 5 to 17 seconds each"
tags:
  - engine
---

# Feature: fast open of a migration log

## Report

From the cat track: opening the campaign app's log in the engine costs about 4 seconds of CPU per migration. Opening release 1.0 takes about 5 seconds, and each release 1.2 or 1.3 test takes 12 to 17 seconds under parallel load. A user with a few dozen migrations would wait minutes to open a database.

Profile an open of the app's release 1.3 log, find where the time goes (checking, admission of constraints, catalog writes, page flushes), and bring an open of an already-applied log down to well under a second per migration.

## 2026-10-27: reverts

Each open that applies a revert takes about 2 seconds in the schema operator law sweep, which takes about 11 seconds for two walks.

## 2026-10-27: campaign port

After the port, a fresh campaign database opens through all four migrations in about 13 s, against about 4 s at one, so each app test takes 12 to 20 s and three pass the 20 s slow mark.
