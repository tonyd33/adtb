---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 96
summary: rerunning conformance/gen/queries/build.sh turns a passing case of 06-forms.test back into a stale refusal
branch: adtb-qtest
tags:
  - quality
  - test
---

# Bug: regenerating the query conformance files restores a stale expectation

## Report

`conformance/queries/*.test` are generated from `conformance/gen/queries/bodies/*.test.in` by `conformance/gen/queries/build.sh`, but `fix(check): pair constructor fields by name, ...` edited the generated `conformance/queries/06-forms.test` and not its body. Running `build.sh` today rewrites `06-forms.test:106-111`, the case "A nullary constructor also takes the payload forms" (`query rowsort` with rows `0`, `1`), back to the old "takes no payload form other than itself" with `error Invalid`, which the fixed checker no longer gives. The other 34 generated files match.

Repro: copy `conformance/gen/queries` to a scratch tree with a sibling `queries/`, run `build.sh`, and diff against `conformance/queries/`.

Fix: apply the edit to `bodies/06-forms.test.in`, and add a test in `adtb-conformance` that expands every body (in Rust, or by running `build.sh` into a temp dir) and fails when a generated file differs, naming the file.
