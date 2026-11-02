# Verification and resumption evidence

## Verification receipt

Record one receipt per check scope in the track's handoff. Link retained
logs rather than copying their output. Include:

- State: workspace path, full tested commit id and base revision. Snapshot
  with `jj status` before the check and record the id with
  `jj log -r @ --no-graph -T 'commit_id ++ "\n"'`. Check the state again
  after the command; report any changes to its inputs during execution.
- Scope: exact command, working directory, relevant toolchain, profile,
  features and environment overrides. Distinguish the root workspace from
  `apps/campaign`.
- Outcome: passed, failed, timed out, interrupted, blocked or not run;
  command exit status when collected; failing tests or incomplete stages;
  retained log path. A successful early stage is not a complete check.
- Timing, when observed: brief-to-publication elapsed time and command
  stages, distinguishing compilation, test execution, review and coordination.
  Separate permission wait and queue/lock wait from execution time. Mark
  unavailable measurements as unknown. Do not attribute a whole tool-call span to execution or infer
  contention from elapsed time alone.
- Remaining work: checks still required and who owns them. Name one owner
  for the final full check. The lead owns post-land tree comparison and any
  checks required by a changed integration delta.

For example:

```text
State: /path/to/adtb-track, tested COMMIT, base BASE; inputs unchanged
Scope: DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target check; workspace root
Environment: toolchain TOOLCHAIN, profiles/features as configured
Outcome: interrupted, exit 143; clippy passed, root tests did not run
Evidence: target/check-RUN.log
Timing: elapsed DURATION; permission/queue/execution unknown
Remaining: independent review, final full check by assigned owner, lead tree comparison
```

A passing final full check certifies its tested tree, regardless of commit
identity. After landing, record `jj diff --from TESTED --to main --summary`.
An empty diff reuses the receipt without another full run, provided the
recorded toolchain, configuration and external test inputs are unchanged.
A changed combined tree after rebase or conflict resolution needs delta
review and affected checks. Repeat the full check only when the impact
cannot be bounded, and record the reason. For a prose-only delta, reuse
unaffected check results when tested inputs, including embedded examples
and build inputs, are unchanged; review the delta and run its relevant
checks. Receipt and bookkeeping edits alone do not trigger Rust suites.

An isolated pass does not erase a suite timeout. Report both. Permission
wait remains part of total elapsed time but is not execution time; report
it as permission wait when that cause is established. Never bypass approval
in order to reduce the measured delay.

## Performance iteration

Order development checks by the cost of finding a reason to reject. Before
measurement, declare the candidate, workloads, pair order, stage gates and stop
rules. Run focused correctness and containment checks, build a matched normal
release, then screen the known regression before the primary improvement.
Keep native execution bounds, fresh databases, unchanged logical work and exact
state/reopen oracles in these screens. An unreviewed screen is provisional.

For the current comparative optimization, Stage A is counter N=1000 with four
writers: two adjacent opposite-order pairs. Collect its four observations, then
reject median paired steady slowdown above 5%. Reject completed-call p99 when
both pairs increase and median paired growth exceeds both 10% and 0.25 ms.
These preview the original final limits; do not demand a concurrent speedup
that final acceptance does not require. On rejection, stop and retain the
unstarted rows. Only on pass run Stage B: two opposite-order primary N=1000,
one-writer pairs, both positive with median saving at least 5%. Compare each
stage against the original normal-release baseline. A native, oracle, identity
or context failure stops immediately. No retry, extra point or favorable
resampling. This applies to future candidates; closed cohorts stay closed.

Only a candidate passing both development stages proceeds to complete its
independent source review, meaningful mutation checks, one final full correctness
check and required opt-in package checks. Review may overlap earlier work on an
immutable revision. Final admission still requires the original entirely fresh
80-run confirmation, unchanged acceptance gates and independent raw-data audit.
A screen supplies no final observations or permission to land.

Preauthorize one bounded development trial in its implementation brief, including
its fixed stages, native controls, baseline, evidence and exclusivity conditions.
The assigned owner may freeze the actual new candidate and run the guarded
screen once those conditions pass; a routine build-to-screen transition needs
no additional lead launch round. A collector may provide the existing execution
service under that same authorization. Stop on input, scope, resource or native
failure and report to the lead. No automatic variant loop or favorable repeat.
Root retains new-scope rulings and promotion, final confirmation and landing.
Generate mechanical build/identity evidence with the build job where possible;
use one receipt, rather than serial author/collector/lead prose handoffs.
A failed development screen needs its declared arithmetic and integrity evidence;
another independent raw-data reconstruction is optional diagnostic work. Final
acceptance still requires independent review and complete raw-data audit.

Use one concise iteration ledger linking native artifacts. Reuse unchanged
helper, compiler and control evidence when its actual inputs are identical;
create the full promotion receipt once a candidate survives screening. Record
compilation, test execution, mutation, review, coordination and measurement time
separately when observed. Do not repeat a full suite for each development edit.

Retain the same author, workspace and warm Cargo cache across variants when
ownership is unchanged. Before editing a closed candidate, preserve its immutable
jj revision, standalone executable, build identity and required raw/proof inputs.
Check that mutable Cargo outputs are not retained executables or proof inputs,
including hard links. Active measurement inputs remain frozen until closure.
A build cache is disposable acceleration, never evidence of source identity.
No concurrent writers, blanket cargo clean or deletion of historical evidence.

## Continue a stopped full check

Keep the failed command's exit and the stages it did not reach. When the
correction changes only expectations, fixture data or prose with a proved
bounded consumer set, retain unaffected passes, rerun the complete affected
consumers and finish every unreached configured stage with the original native
commands. Record a matrix linking each final input scope to its command and
exit. Do not relabel the original failed command as passed. Shared production
changes after the failed attempt follow the normal full-check requirement.

## Bounded fixture and diagnostic-tool changes

A case-data or test-driver migration may finish with complete affected-package
checks instead of a full engine run when the lead establishes its boundary:

- Database and language production behavior, dependencies, features, build
  scripts, test containment, global configuration and shared execution
  boundaries are unchanged. A test-only path alone does not establish this.
- Every changed driver and diagnostic-format consumer is identified, compiled
  and checked. Run complete tests for those packages, including the language
  test binary when its internal helpers change, plus affected strict lints
  and applicable static/documentation checks.
- Independent review and preservation/sensitivity evidence cover the migration.
  Record the exact final combined tree, native exit and unchanged inputs.
- Name the earlier full-check receipt reused for unaffected components and
  explain why the changed inputs cannot reach them. Do not claim that receipt
  tested the new cases or drivers.

A production change or an impact that cannot be bounded gets one final full
check after review. Keep such work separate from an independently ready
fixture batch; its longer verification is not a gate for unrelated changes.

## Isolated opt-in leaf tools

A new tool may add private dependencies and an owned build script without
repeating unchanged engine checks when the lead establishes all of these:

- No existing package depends on the tool. Default features keep its optional
  runtime and build dependencies inactive. Advertised opt-in modes are explicit.
- Compare the actual locked, resolved package graphs, including normal, build
  and dev edges, target conditions and features. Every existing package and
  lock record remains identical in default and supported opt-in modes. Review
  host/target context limits rather than calling metadata a compiler unit graph.
- Existing database, language, shared runtime, manifests, profiles, build
  scripts and containment are unchanged. Any independently combined test or
  prose changes retain their own applicable receipts.
- Review the new build script and dependency build effects: generated outputs
  belong to the tool's build directory, and environment/library selection is
  explicit. Run full dependency license, advisory, source and duplicate policy.
- Run complete tool-package checks for default and advertised feature modes,
  strict affected lints and applicable static checks. Check actual linked
  native-library identity where relevant. An empty default test set does not
  certify opt-in code.
- Identify and run affected workspace discovery and containment consumers on
  the combined tree. Default binary discovery does not certify feature-gated
  binaries: retain explicit containment evidence for those too.
- Independent review and an immutable receipt cover the final inputs. Reuse
  unchanged checks only after tree/configuration/environment comparison; do
  not label retained component evidence as a new whole-workspace check.

Record the isolation evidence, direct consumers and final check owner in the
track. Use native Cargo metadata and bounded graph comparisons; expanding every
repeated dependency path is not a useful proof. If isolation is unproved or an
existing dependency, feature, shared source or execution boundary changes, run
the applicable full check and state the unresolved impact. A full default
engine check never substitutes for the tool's supported feature checks.

This extends bounded verification to isolated consumers; it does not establish
benchmark equivalence, crash durability or measurement readiness.

## Durable task state

Keep the scope, acceptance commands, workspace/base, unresolved failures,
landing dependencies and next action in a tracked ticket or supporting
note before relying on it for another machine or harness. Include the
receipt or enough evidence to rerun the checks if local logs are absent.

Verify that the essential paths appear in `jj file list PATH...`; being
present on disk or readable by tix does not prove a file is tracked.
Explicitly track only the needed files with
`jj file track --include-ignored PATH...` when appropriate. Do not unignore
or import the whole local history. Do not publish private transcripts or
credentials in a durable note.

Keep `work/HANDOFF.md` as a short current checkpoint linking to durable
state and any local evidence. Archive superseded narrative under
`work/briefs/` before shortening it. Label unlanded work and incomplete
checks explicitly; preserve source edits and required artifacts.

On resume, read the current user direction and inspect live jj/ticket state.
Historical permissions in a handoff or archive do not grant authorization
to the new session. If ignored notes are absent, use durable state and
report missing evidence rather than inventing a successful check or a task
boundary.
