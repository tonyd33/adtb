---
created-at: 2026-10-25
status: open
kind: feature
priority: 50
summary: "tix is not on PATH in the devenv shell, so ticket commands go through nix run"
tags: [tooling]
---

# Feature: tix in the devenv shell

## Report

`tix` is a flake at `~/code/tql/work/tools/tix`. adtb's devenv does not
provide it, so every ticket command runs as:

```
nix run path:/home/tony/code/tql/work/tools/tix -- -C work/issues check
```

Expected: `tix` on PATH in the devenv shell, with `-C work/issues`
set, as tql's `devenv.local.nix` does.
