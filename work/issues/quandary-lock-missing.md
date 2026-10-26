---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 60
summary: "the installed quandary has no lock or unlock command, so agents edit the design space without the edit lock"
closed: 2026-10-25
tags:
  - tooling
---

# The installed quandary has no edit lock

## Report

quandary's own ticket `quandary-edit-lock` is resolved, but the
`quandary` on PATH (`/nix/store/7zkpmm9pl8hndzny6xr7b5k4rmp2y2l8-quandary-0.1.0`)
lists no `lock` or `unlock` command in `quandary --help`. Agents have
edited `work/research/` without the lock, so a ruling made mid-edit
can read a transient state.

Expected: the installed quandary is updated to a version with `lock`
and `unlock`.

## 2026-10-25: resolved

## 2026-10-25: resolved

The quandary in devenv.lock now has `lock` and `unlock`. Agents that edit the design space wrap the edit in `quandary lock REASON` and `quandary unlock`.
