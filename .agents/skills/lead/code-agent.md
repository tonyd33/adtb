# Common instructions for code agents

Repo adtb: an embedded database with native sum types. Rust workspace,
jj colocated with git. Several agents work at once, each in its own jj
workspace (a separate working copy of the same repo). Edit source and build
only in the workspace your brief names. A brief may explicitly authorize named
report, receipt and handoff files under the main checkout's ignored work/briefs;
that does not authorize its tracked source, tickets or jj state. Never discard
anything; no destructive jj or git commands. Do not land or push. Keep your track as one change: describe it with the subject your brief names, or amend it, and start no others.


Own execution, result assessment and corrections as well as implementation.
Return a candidate with a verification receipt, not only command requests.
Use the authorized execution path. If access actually blocks it, report the
specific operation and observed result, and retain diagnosis/fix ownership
when the lead supplies a temporary command-launching service. Distinguish
intentional scope restrictions from unverified environment assumptions in a
brief; raise the latter rather than treating an old failure as proof that
you cannot execute now.

Read first: your brief, then only the design and spec sections it
names, by heading: `rg -n '^#' FILE`, then `sed -n A,Bp FILE` for
one section. Never `cat` a whole `docs/` file. Read `AGENTS.md` and
`docs/design/README.md` once. Rulings: `quandary rulings | grep -iE
'TERMS'` from inside `work/research/`, with terms from your brief,
never the full list. Do not load the design-space skill or read the
quandary README. Wrap any edit under `work/research/` in `quandary lock
REASON` and `quandary unlock`.

Orientation: run `work/tools/codemap CRATE` from your workspace root
for the crate's modules and sizes, `work/tools/codemap CRATE MODULE`
(a module may be a file: `codemap adtb heap/page.rs`) for a module's
files and pub items with line numbers, and `work/tools/codemap --tests
[CRATE]` for test binaries and their test counts. That replaces
`ls`, `wc -l` and searches for public declarations. If an LSP is
available, verify which checkout it indexes before using its outline,
references or workspace symbols. Use `rg` in your workspace for callers
when symbol tools are unavailable or index another checkout.

Reading code: locate symbols with `rg -n`, then read at most about
150 lines per range. Never `cat` several files in one call. Keep command
output under about 300 lines; redirect larger output to a log and read
the relevant ranges. Check the active harness's output limits rather
than relying on automatic truncation or storage. Avoid rereading material
already in context. Batch independent reads in one call.

Tiers: `docs/spec/` is what users observe; `docs/design/` is the
contract between subsystems; everything else stays inside your crate.
Depend on another subsystem only through what its design file states.
If you need something a design file does not state, write it into that
design file as a contract (precondition, postcondition, invariant) in
the house style, and report it; do not reach into another crate's
internals. Never patch `docs/spec/` to fit your code: report a spec gap
with a concrete program or history instead.

Dependencies: `deny.toml` allows only the crates listed in
`[bans].allow`. Adding a crate needs an entry in `[bans].allow` with a justification comment, MIT-compatible
licence; only add crates your brief approves, and report any other you
think you need instead of adding it.

House rules for code: no comments by default (only a non-obvious
invariant a reader cannot derive); doc comments in the imperative or
"Returns ...", never restating the name; no em dashes; no spec section
numbers, rule ids, file citations or markdown bold in comments. Match
the style of the existing crates.

Quality rules: Fix the code. Add an `#[expect]` only when the rule is
wrong for that spot, and name it in the report; never add `#[allow]`.
No new function over 150 lines. A panic on an invariant names the
invariant it guards. Use newtype ids (`Xid`, constructor
indexes) rather than bare `u64`/`u32`. Reuse what exists before
writing a helper: the byte readers in `storage/header.rs` and
`storage/record.rs`, and the simulator test-support library
`crates/adtb-simkit`. Every loop over seeds puts the seed in
its assertion message. Never discard a `Result` with `let _ =`.

Tests: cover the agreed behavior, including regressions that currently fail.
Preserve evidence of unexpected failures. Apply AGENTS.md's Test value rules
before changing implementation: correct code that violates its contract, and
correct, replace or remove tests with wrong expectations or unjustified internal
pins while preserving valid coverage. Never weaken a test to conceal a contract
violation. Raise ambiguous contracts or architecture-limiting compatibility
requirements to the lead with a recommended disposition. Record material
failure decisions in the existing receipt, not a separate report per failure.

Your workspace may be a reused slot: its `target/` holds earlier builds, and
a file outside a commit and `target/` stops the next track reusing it. Build
only into your workspace's own `target/`. Keep that warm cache when continuing
variants of the same track. A brief may explicitly lease a different retained
Cargo cache to one owner after checking no other actor uses it; never treat
mutable cache outputs as frozen executable or proof inputs. Preserve the closed
candidate's immutable revision, separate executable and required evidence before
changing source or reusing its cache. Never set
`CARGO_TARGET_DIR` under /tmp or the scratchpad: /tmp has a small
quota, and filling it breaks every agent's shell. Run
`work/tools/workspace-gc --guard` before the full check. Below its free-space
floor, report the resource gate to the lead, who coordinates native idle-build
cleanup under the lead workflow. Do not run `cargo clean`; preserve verification
artifacts outside debug/tmp and builds another actor still uses.

Shell: zsh. Quote globs in arguments (`grep -rn PAT crates
--include='*.rs'`) or use `rg PAT crates`; an unquoted
`--include=*.rs` fails with `no matches found` and prints nothing
else. A variable holding a command does not word-split; define a
function. `grep` is ugrep. Your workspace is jj only: `git` fails
there, so use `jj diff` and `jj log`. Engine integration tests are in
`crates/adtb-sim/tests`; `crates/adtb` has unit tests only. tix:
`direnv exec MAIN tix -C $PWD/work/issues CMD` from your workspace root,
where MAIN is the main checkout (a later `-C` resolves against the
first).

Editing: read the range you will change, about 150 lines, with an
available file reader or bounded shell read. Apply a focused patch with
the available editing tool, respecting any tool-specific read requirement.
A scripted rewrite asserts its pattern matched exactly once.

While iterating, run only the affected test binary (`cargo nextest run
-p CRATE --test NAME FILTER`, or `-p CRATE --lib MODULE::` for unit
tests) and `cargo clippy -p CRATE --all-targets -- -D warnings`. Do not
run `cargo build`: it compiles tests in the dev profile, which nextest
does not use. To check conformance files: `work/tools/conf
conformance/PART/FILE.test` (files or directories; reference and
engine; prints `FILE:LINE: SIDE: difference`). To try a query without
writing a test: `work/tools/adtb-probe --schema FILE [--data FILE]
[--explain] 'TEXT'` prints the checker's refusal with line and column,
or both sides' results and whether they agree. Clippy
denies, among others, `type_complexity` (name a `type` alias),
`too_many_lines` (150), `cognitive_complexity` (25),
`too_many_arguments`, `needless_raw_string_hashes`,
`redundant_closure_for_method_calls`, `match_same_arms`,
`uninlined_format_args` and `self_named_module_files` (a module with
children is `foo/mod.rs`, never `foo.rs` beside `foo/`).

During implementation, run targeted checks and named mutation tests.
For a code track, the brief names one owner for the authoritative `check`
on the final reviewed tree. Run it before reporting only if you own it;
otherwise report targeted results and the remaining owner.
From your workspace root in the dev shell:

```sh
DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check && work/tools/house-check
```

The explicit roots keep generated dev-shell wrappers on your workspace.
`check` owns root clippy, formatting and the suite; campaign lint parity,
clippy, formatting and tests; dependency policy and source-size limits.
Report each failed or unfinished check. The lead compares the landed tree
with the tested revision and reuses the receipt when inputs are unchanged;
landing alone does not require another full check.
Record a [verification receipt](verification.md#verification-receipt) with
its tested state, command/scope, outcome, log and remaining owner. Separate
observed permission wait from execution and total elapsed time; mark timings
unknown when the harness cannot measure them.
For prose-only changes, use the brief's checks and review the diff.

Do not filter a running check through `grep`, `head` or `tail` for its
status. Redirect output to a log, capture the command's exit status before
inspecting that log, and return that status. For example, in a subshell:

```sh
(
  mkdir -p target || exit 1
  if DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check >target/check.log 2>&1; then
    check_status=0
  else
    check_status=$?
  fi
  tail -80 target/check.log
  exit "$check_status"
)
```

`work/tools/suite` builds the
tests, then waits for the machine lock, which one test run or `check` holds
at a time.
It prints failures, timeouts, the summary and the output path
(`target/suite.txt`, ending in `suite: exit N`). Use the harness's
asynchronous execution when available, collect the final exit status and
inspect the output. If it returns a session handle, wait on that handle;
if it provides completion notifications, use them. Avoid busy polling.

Before repeating a command, preserve its canonical output artifacts at unique
stage paths as well as stdout/stderr. Some tools keep failure details in files
that the next invocation overwrites. For suite, preserve target/suite.txt before
running the next case or mutation.

For a timeout, report the test, suite conditions and any isolated outcome.
An isolated pass leaves the cause unresolved unless other evidence proves
it; it does not make the full suite pass. Do not repeat the full suite
without a specific diagnostic reason. `house-check` prints added lines
that break a comment rule and lists added plain `//` comments; retain only
comments stating a non-obvious invariant. Report its result.

Keep detailed verification in one canonical receipt, linking its raw logs and
matrix. Summarize it in the report and link it from the handoff; do not copy the
same command history into all three artifacts. Publish source and machine
evidence promptly so prose preparation does not serialize review.

Report: files changed, what works, tests and which fail, contracts you
wrote into design files, spec gaps, and anything you stopped on. Plain
prose. End with a handoff note of at most 15 lines: the files and
functions a successor must know, with line ranges, verification receipt
or link, incomplete checks and next action. End the assigned turn with the
report. The lead may resume the same author for corrections or a related
cohort; a replacement reads the handoff before taking ownership.

## Tickets

Work is tracked as tix tickets in the main checkout's
`work/issues/` (the `default` row of `jj workspace list`). Read the tickets your brief names
there (their `## Report` states the gap). Do not edit tickets; the
lead updates them. Report anything that should become a new ticket.

Default test cases to runtime-loaded fixtures. Keep a small shared Rust
driver where it is needed to exercise APIs or check typed/internal properties;
that does not justify compiling each input and expected outcome into Rust.
A retained or new compiled case needs a reason that runtime data with an
existing or reasonably small shared driver cannot preserve its assertions.
Convenience or the absence of an existing runner feature is not sufficient:
compare a reusable driver extension with retaining the compiled case family.
State justifications by family in the brief or receipt, not boilerplate comments.
Load case data at runtime; embedding it or generating Rust cases retains the
compilation dependency this policy is intended to remove.

Test value and cost: use retained timings to find expensive tests before
adding coverage. Remove redundant tests only after naming the retained
stronger coverage. Remove pins on details no contract states; repair a weak
oracle when its behavior has no other coverage. Report measured before/after
cost with the same test profile and search depth. Do not keep a test merely
because it already exists, or discard a distinct contract merely because it
is expensive.

Test time: a new test runs in under 2 seconds in the default nextest profile unless it is a sweep; a sweep reads its seed count from the one place the suite states it, keeps a small default count, and runs the full count only under a deep profile. Report the time your new tests add to the default run.

## What gets committed

The repository is public, `work/issues`, `work/kb` and `work/research` included. Leave credentials,
personal data and machine details out of commits and tickets. Use relative paths such as
`../adtb-slot-N`. `work/briefs/` is local.

## Saving time

Use targeted suites during each round; reserve the full check for the final
reviewed tree and its assigned owner. For performance tracks, follow the staged
screen in verification.md: known regression first, primary improvement next,
then expensive mutation and final verification only for surviving candidates.
Do not use a provisional screen as correctness certification. Repeat checks only
for changed inputs, failures or unresolved concerns. Run `work/tools/suite` in the background:
it can wait out a whole `check`, and a foreground call with a harness timeout
dies waiting. Run each mutant against the test expected to kill it, by name.

## Scripts

Keep your scripts, mutation drivers and their logs under your workspace's `target/`. A scratchpad
directory may be shared with other agents in the session: a script there can be overwritten by
another track, and running it can change that track's workspace.

Never use `pkill -f` or a broad process pattern. Kill only a PID you started.

## Memory

A test binary aborts past 1.5 GiB of live heap, charged by malloc chunk, by a counting allocator (`crates/adtb-heap-cap`) that dev-dependencies link in; a test binary run directly is capped too, as are the binaries tests spawn and `work/tools/adtb-probe`. A test that needs more calls `adtb_heap_cap::raise_cap(BYTES)` and declares `threads-required` in `.config/nextest.toml`, the cap over the default rounded up. `ADTB_TEST_HEAP_CAP` overrides everything, in bytes. The cap is per process: run tests through nextest, since plain `cargo test` shares a process across a binary's tests and can abort falsely; use `RUST_TEST_THREADS=1` for a direct run. Page memory and C allocations are not counted. `cargo build --workspace` and `cargo run -p` have no cap; any build with `--all-targets` or `--all-features` does, and a `target/debug/adtb` it leaves aborts at the cap and obeys the variable until rebuilt. Run a text that can grow without bound (a doubling text, a huge file) only as a test or through `adtb-probe`. `ulimit -v` and `ulimit -d` do not work: page reservation fails under them. A larger stack in a probe does not make a runaway finish.

## Waiting

Never run a command that waits on input it will not get. Never poll with an unbounded loop: run the command in the foreground, or in the background and wait for its notification. A wait for something else must have a time limit and must fail when the limit passes. An agent stuck this way gives no signal, and two such shells once held a track for 80 minutes.
