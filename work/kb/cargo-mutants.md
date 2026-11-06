# cargo-mutants

kind: system
home: https://mutants.rs

## Selection

- `--in-diff FILE` keeps a mutant when its span overlaps a changed line on the new side of the
  diff. A signature change selects the whole-body replacement of the function; a hunk of only
  removed lines inside a body still selects that body's replacement; a doc-comment-only hunk
  selects nothing. [V 2026-11-06] cargo-mutants 27.1.0, `--list --in-diff` on hand-made patches
- The new side of the diff must match the tree it runs in. [U]
- Inside a macro invocation it generates no operator mutants of the arguments; a macro call used
  as a match guard is replaced whole with `true` and `false`. [V 2026-11-06] cargo-mutants
  27.1.0, `--list` over a match guard wrapped in a macro call with the comparison as an argument
- A function returning a type with no `Default` gets `Ok(Default::default())`-style mutants that
  fail to build and count as unviable. [V 2026-11-06] cargo-mutants 27.1.0 run

## Execution

- `--in-place` mutates the source tree itself and refuses `--jobs`; without it each job copies
  the tree into `TMPDIR` and builds in the copy. [V 2026-11-06] cargo-mutants 27.1.0 argument
  error `the argument '--in-place' cannot be used with '--jobs <JOBS>'`, `--help`
- Each mutant has a Build phase (`cargo nextest run --no-run --package=MUTATED`) and a Test
  phase; packages and targets added through `--cargo-test-arg` are compiled only in the Test
  phase, so their build time counts as test time. [V 2026-11-06] cargo-mutants 27.1.0 logs
- `--test-package A --test-package B` and `--test-package A,B` both resolved to A alone in a
  workspace run with `--in-diff`; `--cargo-test-arg=--package=B` added B. [V 2026-11-06]
  cargo-mutants 27.1.0 debug.log `test_packages=Explicit([...])`
- With `--test-tool nextest` the repository's `.config/nextest.toml` applies and nextest's
  fail-fast stops a caught mutant's run at its first failure. [V 2026-11-06] cargo-mutants
  27.1.0 logs
- The baseline sets the test timeout automatically, about five times the baseline test time
  (320 s baseline, 1603 s timeout). [V 2026-11-06] cargo-mutants 27.1.0 log
- Output under `--output DIR/mutants.out`: `caught.txt`, `missed.txt`, `unviable.txt`,
  `timeout.txt`, `outcomes.json` with per-phase durations, `log/` per mutant. [V 2026-11-06]
  cargo-mutants 27.1.0 run
- `--shard K/N --sharding round-robin` runs mutant i when i mod N is K. [V 2026-11-06]
  cargo-mutants 27.1.0 `--help`
- It takes no lock of its own beyond `mutants.out/lock.json` in its output directory. [U]
