---
created-at: 2026-10-31
status: in-progress
kind: bug
priority: 403
summary: suite combines implicit workspace selection with explicit package selection and starts unrelated test builds
branch: suite-package-selection
---

# Targeted suite package selection

`work/tools/suite -p adtb-lang --lib translate::tests::` started unrelated
workspace library compilation during the pinned-field fix. The author
interrupted that run and executed an exact-package command instead. The
wrapper currently adds `--workspace` to both build and execution phases.

Preserve native caller-selected package scope in both phases, while keeping
the default full-workspace run and explicit workspace requests. Preserve
arguments, lock ownership, failure propagation and result reporting.
Verify package-scope selection against Cargo/nextest's native semantics
and exercise the tool boundary with shell fixtures; no extra Rust rebuild
or a new full-suite run is required for argument-forwarding evidence.
Keep this source-isolated from the active combined integration check; land
after that checked stack and use affected tool checks for this bounded delta.

## 2026-10-31: reviewed candidate ready

Candidate `149817d77f8c` removes the forced flag from both phases and
forwards caller arguments unchanged. Native offline metadata confirms
all 17 members are default members; exclusion requires explicit
`--workspace`, now documented. Fresh independent review found no blocker.
Twelve shell cases passed in 1.129 seconds; build-only, run-only and both
forced-workspace mutations failed their intended scope assertions. Syntax,
shellcheck and house checks passed without Rust compilation.

Land after the combined checker/pinned stack finishes its separate final
check. Reuse unaffected Rust results and the tool's affected shell receipt
for this bounded forwarding delta; do not launch another full Rust run.
