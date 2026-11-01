---
created-at: 2026-10-31
status: open
kind: bug
priority: 570
summary: a refused host open leaves its newly created file when the filename is not UTF-8
tags:
  - quality
  - engine
---

# Open acquisition create cleanup

## Report

Read-only candidate from the fault-open review on `dffba25e`, also
present on base `38f07e0e`; not executed. On Unix, an absent filename
containing a non-UTF-8 byte can be created by `api/file.rs::open_file`.
`Held::attempt` then canonicalizes it and refuses when
`file_name().to_str()` returns `None`. The resulting `OpenError::Io`
has `InvalidInput`, but the newly created empty file remains.

The refusal occurs before a `Held` is returned. `Database::open` uses
`Held::take(...)?`, so its cleanup match is never reached and the
creation fact is lost with the failed acquisition. This contradicts the
failed-create cleanup postcondition in `docs/design/server.md` section
4.1. The bounded fallback-provenance fix does not introduce this path
and does not repair it.

Reproduce through actual `Database::open` with a scratch Unix path built
using `OsStringExt::from_vec`, create enabled, and an otherwise valid log.
Assert the exact refusal category and that the attempted file is absent;
keep an unrelated sentinel intact. No log is expected to have been
claimed because the runtime has not been constructed. Do not infer a
passing public-open result from a low-level acquisition test.

Class: multi-step change not atomic under failure. The operation's
creation fact must survive until failure cleanup. Reject unsupported path
shapes before creating anything where possible, and inspect other errors
after successful creation before selecting a general acquisition cleanup
boundary. A fix only for this filename shape must say which post-create
errors remain. Fold this host acquisition stage into fault-matrix coverage.
