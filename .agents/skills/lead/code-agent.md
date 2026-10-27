# Common instructions for code agents

Repo adtb: an embedded database with native sum types. Rust workspace,
jj colocated with git. Several agents work at once, each in its own jj
workspace (a separate working copy of the same repo). Work only in the
workspace your brief names. Never discard anything; no destructive jj
or git commands; do not commit.

Read first: your brief, then only the design and spec sections it
names, by heading: `grep -n '^#' FILE`, then `sed -n A,Bp FILE` for
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
`ls`, `wc -l` and `grep 'pub fn'`. For one file's full outline, the LSP
documentSymbol call works in your workspace; LSP references and
workspace symbols answer against the main checkout, not your workspace,
so use `grep -rn` for callers.

Reading code: locate with `grep -n` (symbols, `pub fn`, `impl`), then
read at most about 150 lines per `sed -n`. Never `cat` several files
in one call. Keep every command's output under about 300 lines; output
over about 30KB is saved to a file, and reading it back costs the
same. Do not re-read code you just edited. Put independent reads in
one call. Context is re-read on every turn, so each extra kilobyte is
paid for the rest of the session.

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

Quality rules: never add `#[allow]`; use `#[expect(lint, reason =
"...")]`. No new function over 150 lines. A panic on an invariant
names the invariant it guards. Use newtype ids (`Xid`, constructor
indexes) rather than bare `u64`/`u32`. Reuse what exists before
writing a helper: the byte readers in `storage/header.rs` and
`storage/record.rs`, and the test-support modules under
`crates/adtb-sim/tests/support`. Every loop over seeds puts the seed in
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

Editing: read the range you will change with the Read tool (offset and
limit, about 150 lines), then change it with Edit; Edit refuses a file
you only read through `sed`. A scripted rewrite asserts its pattern
matched exactly once.

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

Full check before reporting, run once at the end:

```
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | grep -E '^(error|warning)' -A8 | head -80
cargo fmt --check && echo fmt-ok
cargo deny check 2>&1 | tail -2
work/tools/size-check.sh
work/tools/suite
work/tools/house-check
```

`work/tools/suite` stops below 30 GB free and lists the build
directories (then run `cargo clean` in your own workspace and report
it). It builds the tests, then waits for a machine-wide lock so only
one suite runs at a time, and says so when it queues. It prints only
the failing and timed-out tests, the Summary and the output path
(`target/suite.txt`, ending in `suite: exit N`). Start it with
run_in_background; you are notified when it exits, so do not poll or
sleep. A test that times out in the full run and passes alone is load,
not a failure: report it and do not rerun the suite. `house-check`
prints `FILE:LINE: RULE: TEXT` for each added line that breaks a
comment rule, and lists each added plain `//` comment for you to keep
only if it states an invariant. Report each result.

Report: files changed, what works, tests and which fail, contracts you
wrote into design files, spec gaps, and anything you stopped on. Plain
prose. End with a handoff note of at most 15 lines: the files and
functions a successor must know, with line ranges. Your session ends
with the report; follow-up work goes to a new agent that reads that
note.

## Tickets

Work is tracked as tix tickets in the main checkout's
`work/issues/` (the `default` row of `jj workspace list`). Read the tickets your brief names
there (their `## Report` states the gap). Do not edit tickets; the
lead updates them. Report anything that should become a new ticket.

Test time: a new test runs in under 2 seconds in the default nextest profile unless it is a sweep; a sweep reads its seed count from the one place the suite states it, keeps a small default count, and runs the full count only under a deep profile. Report the time your new tests add to the default run.
