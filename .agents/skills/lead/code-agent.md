# Common instructions for code agents

Repo adtb: an embedded database with native sum types. Rust workspace,
jj colocated with git. Several agents work at once, each in its own jj
workspace (a separate working copy of the same repo). Work only in the
workspace your brief names. Never discard anything; no destructive jj
or git commands; do not commit.

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
wrong for that spot, and name it in the report. No new function over
150 lines. A panic on an invariant
names the invariant it guards. Use newtype ids (`Xid`, constructor
indexes) rather than bare `u64`/`u32`. Reuse what exists before
writing a helper: the byte readers in `storage/header.rs` and
`storage/record.rs`, and the simulator test-support library
`crates/adtb-simkit`. Every loop over seeds puts the seed in
its assertion message. Never discard a `Result` with `let _ =`.

Tests: write the full spec of the feature, including tests that fail;
never delete a failing test; report which fail.

Build only into your workspace's own `target/`. Never set
`CARGO_TARGET_DIR` under /tmp or the scratchpad: /tmp has a small
quota, and filling it breaks every agent's shell. Check `df -h .`
before the full check; below 30 GB free, run `cargo
clean` in your own workspace (and in `apps/campaign` if you built
there) and report it.

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

For a code track, run the authoritative `check` once before reporting,
unless the brief assigns narrower checks and names who owns the rest.
From your workspace root in the dev shell:

```sh
DEVENV_ROOT="$PWD" CARGO_TARGET_DIR="$PWD/target" check && work/tools/house-check
```

The explicit roots keep generated dev-shell wrappers on your workspace.
`check` owns root clippy, formatting and the suite; campaign lint parity,
clippy, formatting and tests; dependency policy and source-size limits.
Report each failed or unfinished check. The lead owns post-land `check`.
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
tests, then waits for a machine-wide lock so only one suite runs at a time.
It prints failures, timeouts, the summary and the output path
(`target/suite.txt`, ending in `suite: exit N`). Use the harness's
asynchronous execution when available, collect the final exit status and
inspect the output. If it returns a session handle, wait on that handle;
if it provides completion notifications, use them. Avoid busy polling.

For a timeout, report the test, suite conditions and any isolated outcome.
An isolated pass leaves the cause unresolved unless other evidence proves
it; it does not make the full suite pass. Do not repeat the full suite
without a specific diagnostic reason. `house-check` prints added lines
that break a comment rule and lists added plain `//` comments; retain only
comments stating a non-obvious invariant. Report its result.

Report: files changed, what works, tests and which fail, contracts you
wrote into design files, spec gaps, and anything you stopped on. Plain
prose. End with a handoff note of at most 15 lines: the files and
functions a successor must know, with line ranges, verification receipt
or link, incomplete checks and next action. Your session ends
with the report; follow-up work goes to a new agent that reads that
note.

## Tickets

Work is tracked as tix tickets in the main checkout's
`work/issues/` (the `default` row of `jj workspace list`). Read the tickets your brief names
there (their `## Report` states the gap). Do not edit tickets; the
lead updates them. Report anything that should become a new ticket.

Test time: a new test runs in under 2 seconds in the default nextest profile unless it is a sweep; a sweep reads its seed count from the one place the suite states it, keeps a small default count, and runs the full count only under a deep profile. Report the time your new tests add to the default run.
