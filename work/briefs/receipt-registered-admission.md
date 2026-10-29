# Registered admission verification receipt

Workspace: `/home/tony/code/adtb-registered-admission`, jj workspace
`registered-admission`; initial `8ff92c6c79a45d6f04dd65f324bf8822216d1159` on
base `39dc440ef0e837f6774fe441e419c2707244a397`.
Frozen source: `a5b883281e07474bc8373dc345d2b9c20deabb09`.
No commits, landing, ticket edits, spec edits, dependencies or API exports.

The writer verified an actual sibling target write/read/remove and shared jj
access with escalation. The main development environment requires escalated
Nix access; all successful verification commands used that environment.

Every Cargo/check command below ran from the assigned workspace through:

```sh
direnv exec /home/tony/code/adtb env DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" COMMAND
```

Root and campaign build artifacts stay in that workspace's `target/`.
Rust 1.98.1 (`48a229ceaefd4985c50990b14116b6d856af0985`), Cargo 1.98.0,
nextest 0.9.146, Nix 2.34.9, Linux x86_64; configured default features/profiles,
nextest `default`. Full toolchain evidence: `target/admission-toolchain.txt`.
All evidence paths below are relative to the assigned workspace.

## Retained failing witnesses

At `ca493bf0cbd82cf7c8209a93d7990e37d54bcdc5`, containing only the original
two new witnesses on the initial engine, command
`cargo nextest run -p adtb-sim --test registered --test bundles admission_`
exited 100: both tests failed. An unused declared I8 argument supplied as
`Value::Int(128)` committed a write and returned `Int(1)`. A wrong-arity refusal
after four simulated hours changed renewal from
`2026-01-01T00:00:00.135841Z` to `2026-01-01T04:00:00.147272Z`.
Evidence: `target/admission-baseline.log`, `target/admission-baseline-state.txt`,
`target/admission-baseline-{registered,bundles}.rs`.

At `9ac7948274ff584c1aa29ba01c5b7b668ca2386c`, before readiness hooks,
`cargo nextest run --no-fail-fast -p adtb-sim --test bundles -E 'test(admission_cancelled) | test(admission_expired) | test(admission_invalid_bookmark) | test(admission_interactive_stops) | test(admission_ended) | test(admission_closed)'`
exited 100: seven selected tests, one passed and six failed. Five failures
reproduced renewal on a pre-cancelled standalone call, expired standalone
deadline, future bookmark, interactive cancelled call and expired transaction
deadline. The closed-database control already passed. The ended-transaction
test initially failed its fixture's result-count assertion, which is not
evidence of an ended-state defect.
Evidence: `target/admission-readiness-baseline.log`,
`target/admission-readiness-baseline-bundles.rs`.

After correcting that resultless-update fixture, production files were saved,
temporarily restored to their retained pre-hook state, and restored in a
`finally` block after this bounded baseline run. At
`daeee04da302cbc4ea5739931c885bcf985aa3b8`, command
`cargo nextest run -p adtb-sim --test bundles admission_ended_transaction`
exited 100: one test failed. The call returned its existing Serialization
error but changed renewal from `2026-01-01T00:00:00.135854Z` to
`2026-01-01T04:00:00.149982Z`.
Evidence: `target/admission-ended-baseline.log`,
`target/admission-ended-baseline-bundles.rs`, `target/admission-hooks-*.rs`.

Iteration evidence remains in `target/admission-focused-{1,2}.log` and
`target/admission-readiness-fixed.log`: initial fixture syntax/result-count
failures were corrected. They are superseded by the frozen checks below.
The first five admission regressions passed before the readiness extension.

## Frozen focused checks

All checks below tested `a5b883281e07474bc8373dc345d2b9c20deabb09` and exited 0.
Before/after status is retained as
`target/admission-{frozen-before,focused-after,final-gates-after}-status.txt`;
full ids are `target/admission-{frozen,focused-after,final-gates-after}-id.txt`.
All tracked input hashes verified unchanged after these checks.

| Command | Result | Log |
| --- | --- | --- |
| `cargo nextest run -p adtb-sim --test registered --test bundles --test principal --test embedding --test api` | 173 passed, 0 skipped; nextest run 6.568s | `target/admission-focused-final2.log` |
| `cargo nextest run -p adtb --lib api::value::tests` | 4 passed, 317 skipped; nextest run 0.011s | `target/admission-validator-final.log` |
| `cargo clippy -p adtb -p adtb-sim --all-targets -- -D warnings` | passed | `target/admission-clippy-final.log` |
| `cargo fmt --all --check` | passed | `target/admission-format-final.log` |
| `work/tools/house-check` | passed, no added plain comments | `target/admission-house-final.log` |
| `work/tools/size-check.sh` | passed | `target/admission-size-final.log` |

The 12 new tests took 0.426–0.656s each in that focused default run;
their summed durations were 5.949s, not additional suite wall time.
There are no new sweeps. Existing lifecycle/current-pin, retry, cancellation,
close, principal and transaction controls passed in the affected binaries.

## Frozen full check

`check` ran once on frozen source, from `2026-10-29T00:35:12Z` to
`2026-10-29T00:42:10Z`. The shell command span was 418 seconds.
Collected session 56248 exit: 0. Before/after full revision ids are identical,
`a5b883281e07474bc8373dc345d2b9c20deabb09`.

Root lint/format/suite passed: 2,426 tests passed and eight were skipped
(nextest run 52.502s; `target/suite.txt` ends `suite: exit 0`). Campaign lint
parity, lint, format and tests passed: 47 tests passed and one was skipped
(nextest run 20.879s). Both dependency policies and source-size checks passed.
Docs passed ten tests, 420 highlighted syntax inputs, 61 HTML pages, 6,734
internal links and 4,999 fragments, with zero failures. Dependency policy
emitted nonfatal unmatched-license/unmatched-skip configuration warnings.

Evidence: `target/admission-fullcheck.log`, `target/suite.txt`, and
`target/admission-fullcheck-{before,after}-{status,id}.txt`.
The wrapper retains exit status, UTC start/end and elapsed seconds in
`target/admission-fullcheck{.exit,-start.txt,-end.txt,-elapsed-seconds.txt}`.
All 2,568 frozen tracked inputs verified unchanged both in the wrapper and in
a subsequent `sha256sum -c target/admission-frozen-inputs.sha256` run, exit 0;
evidence is `target/admission-{fullcheck,final}-input-check.log`.
Post-full `work/tools/house-check` exited 0, with no added plain comments:
`target/admission-house-after-full.log`, `target/admission-house-after-full.exit`.

Permission wait, queue/lock wait and execution-only timings were not measured
and remain unknown. The final wrapper's command span starts after permission
and includes environment setup, queue waits and execution. An earlier lint
iteration visibly waited for a Cargo build lock; its duration is unknown.
No authoritative full rerun was performed. Free space at completion was 35 GB;
this track removed no build artifacts. The lead archived verified fixtures
from completed other workspaces during the check, without changing these inputs.

## Exact inputs and scope

Every tracked file, including root/campaign manifests and locks, devenv inputs,
toolchain and nextest settings, has a SHA256 entry in these retained manifests:

| State | Manifest | SHA256 of manifest |
| --- | --- | --- |
| Original failing baseline | `target/admission-baseline-inputs.sha256` | `01073470cbac031e9bb23e63809861ef37f36bdb6f2107c6a85f5a356d143b6f` |
| Readiness failing baseline | `target/admission-readiness-baseline-inputs.sha256` | `96a5ec6b82c1baf488d7b2fa5f5aefec725308fee2572555ad4230f2add15b4d` |
| Ended-state failing baseline | `target/admission-ended-baseline-inputs.sha256` | `2ba72d19b020f100ff434e24ae8a77b78c6d138b3476aec4f1f34fb93b2c5fdb` |
| Frozen final source | `target/admission-frozen-inputs.sha256` | `bc44c9ea6ccd7002b710ac620f545ac74a370ac9f7466e126bee8abf105c6c84` |

The baseline manifests reuse unchanged frozen entries and obtain all five
modified files from each exact jj revision. The frozen diff is retained in
`target/admission-frozen.diff`; `target/admission-tracked-files.txt` is the
tracked-path inventory. Recreate source and manifests from the named revisions
if local target evidence is absent; rerun the commands above for verification.

Implementation: shared `Registered::admitted` validates every declared argument
through `api/value.rs::valid` with `Entry::types` and `Checked.schema`, before
argument cloning. Existing ordinal/lifecycle/current-pin/principal/arity order
remains; representation checks follow arity and precede read-only refusal.
Malformed argument values are never formatted into errors. Domain predicates
remain write-only; untagged nominal values, absent referents and short/empty
Uuid byte values retain their existing representation semantics.

Renewal now runs through one internal `FnOnce` hook in each owning execution
boundary, after native options/state checks and before statements. Standalone
calls keep their existing outer open guard; interactive renewal executes under
`run`'s existing single guard. There is no additional nested guard, lock policy,
retry policy or transport layer. The hook runs once, before retries; it makes
no promise about late cancellation or execution failures. No new engine/spec
gaps or design contracts were introduced.

Writer verification is complete. Independent review reported matching input
hashes and no source blockers before collecting this full exit; final review
clearance is the lead/reviewer's responsibility. The lead owns ticket updates,
landing and the full post-land main integration check. The branch receipt does
not certify a changed combined tree after landing or conflict resolution.
