---
created-at: 2026-10-25
status: open
kind: feature
priority: 791
summary: "at the prompt a text that parses runs before a continuation line can extend it, the shell has no terminal test, and a violation report prints as a row count"
tags: [engine]
---

# Feature: shell follow-ups

## Report

From the shell track:
- A text that already parses can still be extended (more `| arm` lines of a `match`, an `order by` line, more `transact` statements). Scripts look one line ahead, but at the prompt such a continuation runs as a new text and fails, so a multi-line `match` must be typed on one line.
- No test drives the interactive mode through a terminal; it needs `script` from util-linux or a pty dependency.
- `Violated.report` prints as a row count, since its `Report` encoding is undefined in process (`api-follow-ups`).
- The schema file must be migration syntax with each `add relation` after its type; a declaration file is refused (`prealpha-schema-from-declarations`).

## 2026-10-26: declaration files

A schema file is now accepted by `--schema`, in any declaration order (`shell-usability`). That item is done.
