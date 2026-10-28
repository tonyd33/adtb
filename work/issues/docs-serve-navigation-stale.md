---
created-at: 2026-10-27
status: in-progress
kind: bug
priority: 898
summary: docs serve watches source bodies but never regenerates navigation or copied grammar
branch: docs-watch
tags:
  - quality
  - tooling
---

# Docs serve retains stale generated navigation

## Report

At be6b9b4a, tools/docs-site/site.py:157–161 runs prepare once and execs
mdbook serve. prepare generates SUMMARY.md and copies adt-highlight.js.
Source watching rebuilds page bodies but cannot regenerate these files.

Run work/tools/docs-site serve, add docs/manual/new.md and link it from the
manual README, or reorder that README's links. The generated SUMMARY remains
unchanged: the new page is missing from the sidebar/build and its README link
can lead to a 404 until the server restarts. Grammar edits similarly retain
the copied version. Existing watch verification exercised body edits only.

Regenerate derived navigation/assets during serving when their sources change.
Keep authored Markdown unchanged and add a regression that changes navigation
while the server remains running.
