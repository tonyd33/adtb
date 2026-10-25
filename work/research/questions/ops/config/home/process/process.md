---
title: Process-level settings are not data in the file
pick: any
raised-by: [ops/config/home/relation]
options:
  split:
    title: Only file settings are writable relations; open-time settings are shown read-only
    needs: [ops/config/scope/file, ops/config/scope/open]
  on-commit:
    title: A write to a process setting takes effect in this process after commit
  live:
    title: Live with it
    excludes: [ops/config/home/process/split, ops/config/home/process/on-commit]
---
A pool size written in the file would apply to every process opening
it, on any host; a write that aborts must not have resized the pool
already.

## ops/config/home/process/split

```
relation sys::settings        : Setting                -- in the file, transactional
relation sys::process_options : ProcessOption volatile -- this process, read-only
```

## ops/config/home/process/on-commit

The write commits a desired value; the process applies it after
commit. Another process opening the file applies it too, which may be
wrong for its host.

## ops/config/home/process/live

Process settings are stored in the file and apply wherever it is opened.
