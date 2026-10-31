---
created-at: 2026-10-31
status: open
kind: bug
priority: 560
summary: a create racing an unlink reports its newly created file as preexisting and leaves it after refusal
tags:
  - quality
  - engine
---

# Open fallback create provenance

## Report

Read-only candidate from the quality review through `8538911c095c`, not yet
reproduced. `crates/adtb/src/api/file.rs:28-40` first tries `create_new`
when creating, then falls back after `AlreadyExists` to an open with
`create(true)`, whose result always carries `created = false`.

A concurrent opener holding the existing file's lock can unlink that
file between the failed exclusive create and the fallback open. The
fallback then creates the replacement itself but reports it as
preexisting. If the migration or registration subsequently refuses,
`crates/adtb/src/api/mod.rs:611-616` skips `remove_created` and leaves the
newly created file and its claimed log. This contradicts the cleanup
postcondition in `docs/design/server.md` section 4.1, even when every
unlinker holds the old file's lock and all cleanup operations would
succeed.

The existing `Held::take_opened` hook runs after `open_file` returns,
so `a_path_unlinked_before_the_lock_is_created_again` does not reach
this window. A regression must remove the preexisting path after the
first `AlreadyExists` and before the fallback, observe creation
provenance, then drive a refused `Database::open` and assert that its
new file and log are absent while an unrelated existing file is intact.

Fix direction: opening an existing file must not also create one. If
the noncreating open finds the name gone, retry the exclusive-create
protocol and retain the result of the operation that actually created
the inode. Class: multi-step change not atomic under failure or race;
the ownership fact must come from the creating operation. Include this
host filesystem window in fault-matrix's public-open coverage.
