---
created-at: 2026-11-07
status: open
kind: bug
priority: 17
summary: "a global gitignore entry for work hides new files under work/tools, so a track's new tool lands untracked and check fails after landing; land does not refuse it"
tags:
  - class-none
---

# land misses new files the global gitignore hides

## Report

## 2026-11-07

The maintainer's global gitignore ignores `work`, so every new file under work/ is ignored by jj's snapshot unless tracked with `jj file track --include-ignored`. AGENTS.md says so for tickets only. Test-surface T1 and T2 added work/tools/feature-check and feature-check-test; both stayed untracked, the change landed without them, and `check` on main failed with exit 127 at the `feature-check` step. The slot's `check --changed` passed because it ran main's devenv, which had no such step yet, and the lead ran the tool by hand from the slot's directory, where the untracked file existed.

Direction: `land` refuses when the workspace holds ignored files under directories the repository tracks files in (work/tools, work/issues, crates, apps, docs), naming each and the `jj file track --include-ignored` command; a slot-test case shows it.
