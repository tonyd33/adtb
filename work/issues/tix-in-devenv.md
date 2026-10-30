---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 50
summary: "tix is not on PATH in the devenv shell, so ticket commands go through nix run"
closed: 2026-10-26
tags:
  - tooling
---

# Feature: tix in the devenv shell

## Report

`tix` is a flake at `~/code/tql/work/tools/tix`. adtb's devenv does not
provide it, so every ticket command runs as:

```
nix run path:../tql/work/tools/tix -- -C work/issues check
```

Expected: `tix` on PATH in the devenv shell, with `-C work/issues`
set, as tql's `devenv.local.nix` does.

## 2026-10-26: resolved

## 2026-10-26: landed

tix is published at github:tonyd33/tix. devenv takes it as an input and wraps it as a `tix` script with `-C work/issues` set; outside the shell, `nix run github:tonyd33/tix -- -C work/issues <cmd>`. CLAUDE.md, the README and the quality-review skill use the new form.
