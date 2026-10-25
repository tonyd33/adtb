---
created-at: 2026-10-25
status: open
kind: feature
priority: 175
summary: "read_parquet and read_csv are missing from the prelude, so 4 spec blocks fail"
tags: [lang]
---

# Feature: file scans in the prelude

## Report

The prelude has no `read_parquet` or `read_csv`. 4 blocks fail
`spec_examples_check`: `interop.md:254`, `interop.md:313`,
`interop.md:363`, `writes.md:315`.

```
cargo test -p adtb-lang --test spec_corpus spec_examples_check
```

Actual:

```
interop.md:254:
  interop.md:257: no `read_parquet` in scope
interop.md:313:
  interop.md:317: no `read_csv` in scope
interop.md:363:
  interop.md:369: no `read_csv` in scope
writes.md:315:
  writes.md:320: no `read_csv` in scope
```

Expected: every block checks.
