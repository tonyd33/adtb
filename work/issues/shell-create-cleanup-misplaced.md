---
created-at: 2026-10-27
status: open
kind: bug
priority: 873
summary: "the shell removes a refused create's file and log by rebuilding the pager's private log directory name, and reports a second NotFound error when nothing was created"
tags: [quality, engine]
---

# A refused create in the shell prints a spurious second error, and other clients keep the half-made file

## Report

On a refused `--create`, `crates/adtb-shell/src/main.rs:119-128` and `:157-172` delete the database file and its log directory, rebuilding the `{name}-log` naming that is private to `crates/adtb/src/storage/pager.rs:327`. No design file states that name. `remove` treats NotFound on the file as an error.

```
adtb --create --schema u.adt /nonexist/db -c x
```

Actual:

```
/nonexist/db: the file does not exist
/nonexist/db: No such file or directory (os error 2)
```

Expected: the first line only.

The probe, apps/campaign and other API clients still leave a half-created file after a refused create.

Fix: have `Database::open` with create remove what it made when it fails, stated as a postcondition in the api design file, and drop the shell's `log_dir` and `remove`. At least, tolerate NotFound.

## 2026-10-27: ruling

`Database::open` with create removes the file and log it made when the open fails, and leaves nothing behind; state it as a postcondition in the API design file. The shell then drops its own cleanup and its second error.
