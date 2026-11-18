---
created-at: 2026-11-18
status: open
kind: bug
priority: 650
summary: "cargo conformance --engine --update without --file-name is not refused: the refusal reads file_name.is_empty() after the early return that skips it"
tags:
  - quality
  - tooling
  - test
---

# cargo conformance --engine --update is refused only with --file-name

## Report

`selections` in `crates/adtb-conformance/src/runner/cli.rs:171` returns early when no `--file-name` is given, so the refusal at cli.rs:202-212 (`--update` with only the engine on a conformance file exits 2) runs only for an explicit path. Its `run.file_name.is_empty() ||` term (cli.rs:202) can never be true where it is read.

```
cargo conformance --engine --update
```

Actual: no usage error. Every engine script with a row difference is rewritten, and every conformance file with an engine difference fails without a write, exit 1.

Expected: exit 2 with "`--update` takes the reference's rows; the engine never writes the expected rows of a conformance file", as `cargo conformance --engine --update --file-name conformance` does. `tests/cli.rs` `update_with_only_the_engine_is_refused_for_conformance_files` covers only the `--file-name` form.

The `--update` help text (cli.rs:61-64), "the reference's for a conformance file, unless only the engine runs", reads as if the engine's rows were written when only the engine runs.

Fix: decide the refusal from the selections after both branches (any selected case of `Format::Conformance`), drop the dead term, add the bare `--engine --update` case to tests/cli.rs, and reword the help to say an update never writes the engine's rows into a conformance file.
