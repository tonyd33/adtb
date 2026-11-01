---
created-at: 2026-10-31
status: open
kind: bug
priority: 399
summary: Pinned upserts retain nonhiding admission and nested value-reader gaps
---

# Pinned writes past pin

## Report

## Report

All from reading in the review of staged-operators, except the first item, which was run. All on main before that track. Cause: a pinned translation types a value by the pin although its source ranges wider. Class: none listed fits exactly; closest is authority per entry path, since the gate in `Reads::facts` records an upsert only as a write.

- Upsert `old` on a Link row without the field: refused with `Internal: constructor 1 of the type has the field`, row unchanged. A legal call gets an `Internal`. Tests: `an_upsert_old_*` in crates/adtb-sim/tests/bundles.rs.
- `update old set new` by a pin overwrites a row of a hidden constructor.
- In a `hide new constructors` bundle, a stored value-sum field read through an ordinary scan (`x.tone.depth`) has the same shape: its source is wider than the pin.
- `Walk::field` takes the first owner, so a same-named field on a removed constructor refuses falsely.

Decide the rule first: does the gate count a conflict probe as a read, or does a pinned write refuse a row past its pin with a named error? Run the second and third items before fixing.

## 2026-10-31: executable probes resumed

Workspace `pinned-probes`, based on `fbb17db3`. Run the hidden-row
overwrite and ordinary nested-field scan through public bundle
registration and calls before implementing a fix. The ordinary-scan
finding remains unconfirmed: the reader analysis appears to record its
field reachability correctly.

Existing evolution sections 7.1, 7.2 and 8.1 already require a hidden
conflict to fail by key and preserve the row, and give nested value-sum
readers no hiding exemption. These cases need no new design ruling.
Whether a nonhiding conflict probe counts as a reader still needs a
decision against the adopted rules before changing its admission.

Acceptance for this evidence track: targeted `cargo nextest run -p
adtb-sim --test bundles` filters for the existing `an_upsert_old_`
cases and the new overwrite and nested-field probes. Record exact
refusals and surviving rows, including a failing contract regression;
no production fix or full check belongs to this probe track.

## 2026-10-31: hidden overwrite reproduced; ordinary scan disproved

The public bundle-call probe returned success and replaced
`Link { id: notes.key(1), url: "u" }` with
`Plain { id: notes.key(1), body: "x" }` under `hide new constructors`.
The contract regression asserts a key refusal and the exact unchanged
Link row, and fails on the current engine.

The exact ordinary `x.tone.depth` scan correctly refuses registration
with `BundleStale` for `Tone.Dye`; enabling Dye with that bundle live
also refuses with the readers-first reason. The ordinary-scan candidate
in the original report is disproved on this tree. The separate nested
upsert characterization still succeeds and remains a reader-facts gap.

Commands: `cargo nextest run -p adtb-sim --test bundles an_upsert_old_`
passed all four existing characterizations. The three new
`a_hiding_pinned_` probes yielded two passes and one expected failure
(exit 100). Workspace `pinned-probes` contains the tests in
`crates/adtb-sim/tests/bundles.rs`; preserve the failing hidden-conflict
regression. Next: enforce hidden conflict visibility at the shared
upsert boundary, then independent review and integration checks.

## 2026-10-31: bounded hidden-conflict fix under verification

Workspace `pinned-probes` implements the hiding-only source boundary:
the translated old binder retains its checked constructor scope,
mapped by catalog identity and intersected with the current probe
scope. Engine and reference consumers do not bind an out-of-scope
conflict; they contribute the ordinary proposed insertion, preserving
statement-level validation and error selection. Nonhiding admission,
nested upsert reader facts and the first-field-owner candidate remain
outside this slice.

The public hidden-key, branch, bulk-refusal and rename/removal probes
pass. A direct translation counterexample proved that filtering only
new constructor identities was insufficient when a known constructor
newly joined a scoped unique: the old scope wrongly grew from Plain
to Plain|Other. Mapping the checked binder's original selection fixed
both new-constructor and existing-constructor cases, with hiding and
nonhiding controls.

Independent reading review found no introduced data-loss/wrong-answer
blocker. Its runtime/coverage limits remain explicit. The public
unscoped-unique assertion retains the normative name but is ignored
for the preexisting defect in unscoped-unique-canonical-name. A scoped
public fixture was blocked by named-set-index-scope-resolver; a later
forward-name workaround was invalid and correctly refused. Direct
translator and reference-consumer tests provide separate layers;
they do not claim a complete public scoped-unique integration run.

Remaining acceptance: full affected bundles/reference/translation
binaries, exact guard mutants, clippy/fmt/house, rebase after
test-audit-b lands, and full integration check. Keep the current
unsupported structural-translation refusals. The ticket remains open
after this slice for the reader-analysis and first-owner work.

## 2026-10-31: hiding conflict boundary landed

Landed as `561c5ca496f5ee53122afb42acd51e30d0c9d1d5`, based on
`472200f6`. A hiding bundle can no longer bind a conflicting row outside
its checked constructor scope or overwrite it through the conflict
branch. Translation maps the checked scope by catalog identity; engine
and reference enforce that scope before binding the row. The complete
key/unique probe and ordinary statement validation remain responsible
for detecting collisions and selecting the refusal.

Cause: the current probe range replaced the checked branch source
range, and consumers bound every returned row. This is a source/type
scope disagreement within the copied-rules class. Enforcement is the
typed constructor selection plus a check in each kernel consumer;
stored runtime rows require that check. Public branch and identity
matrices search the admitted domain. Unsupported structural
translations retain their existing refusals.

Independent reading review found no introduced blocker. All three
deliberate mutants compiled and failed their exact regression assertions:
removing the engine guard reproduced successful hidden-row overwrite;
removing the reference guard ran the forbidden branch in key and scoped
unique cases; removing the translator intersection widened the old
binder while leaving the complete unique probe unchanged. Each source
was restored, and all corresponding honest tests then passed.

The final full `check` passed on the unchanged landed source: 3754 root
tests, 48 campaign tests, 13 Loom tests and 39 Miri tests, plus all
configured static, documentation, grammar and tooling checks. The root
suite skipped eight tests, including the normative public unique case
blocked by unscoped-unique-canonical-name. No complete public scoped
unique integration result is claimed. Direct translation and reference
consumer checks supply separate evidence; no physical-undo mutant is
claimed for the bulk atomic-refusal test.

Acceptance commands: `check`; targeted `work/tools/suite -p adtb-sim
--test bundles a_hiding_pinned_upsert_conflicting_with_a_hidden_row_fails_by_key`;
`work/tools/suite -p adtb-lang --lib
a_hiding_upsert_keeps_its_visible_scope_distinct_from_its_unique_probe`;
and `work/tools/suite -p adtb-reference --test writes`.

The lead post-land full `check` also passed on an identical tree, with
the same test counts and all configured checks. Root ran in 205.736s
and campaign in 30.447s; these are run observations, not a performance
claim. This ticket stays open for
nonhiding reader admission, nested-upsert value-reader facts and the
first-field-owner candidate. The ordinary nested-field scan candidate
was disproved; it is not an outstanding defect.

## 2026-10-31: parallel field-owner follow-up

Workspace `pinned-field-owner` owns reproduction and, if confirmed, a bounded
fix for translation choosing a removed first owner of a same-named field.
Preserve catalog field identity across surviving constructors; never substitute
a spelling lookup or choose an arbitrary surviving owner when images differ.
Search removal/declaration-order/rename combinations with explicit expected
values and a mutation that restores the faulty owner selection. Targeted
checks and an independent review precede one final full check.

This track does not decide nonhiding conflict-probe admission or close the
nested-upsert reader-facts gap. It changes translation and its focused tests;
the concurrent checker migration changes checker test data/driver only.

## 2026-10-31: combined final verification plan

Both implementation tracks have completed targeted verification and have
separate fresh independent reviews. After review convergence, stack the
pinned-field fix before the checker migration on current main. The checker
author owns one authoritative full check of that combined final tree; each
author retains diagnosis and corrections for their own change. Native
landing preserves both commits and reuses the receipt only after comparing
the landed tree with the tested combined tree. Do not run a separate full
check merely because each change has its own ticket.

## 2026-10-31: field-owner identity fix landed

Landed `efb953db9bba`. The valid public reproduction reads shared fields
through a proper constructor subset; the initially suggested whole-sum
query is refused by the checker and did not establish this defect. The
translator formerly chose the first spelling owner even if removed. It
now maps each surviving owner's catalog field identity, retaining a direct
projection only when images agree and lowering different renamed images
through existing typed constructor arms.

Six runtime cases preserve pre-migration rows across both declaration
orders, removal, survivor rename and independent owner renames. The
original selection mechanism failed the exact public registration
regression; byte-restored healthy code passed. Fresh independent review,
17 translation tests, 123 bundle tests, and affected lint/format/house/size
checks passed. The new public matrix cost about 1.2 seconds.

The single combined final check on `047020687851` passed with the checker
migration: 3650 root, 48 campaign, 13 Loom and 39 Miri tests plus all
configured supporting checks. Existing skips remained. Own exit was zero,
revision/status/all 3094 tracked inputs were unchanged, and the landing
delta contained only reviewed ticket prose. No duplicate full check ran.

This umbrella remains open for nonhiding upsert admission and nested-value
reader tracking through upserts. The ordinary nested scan was disproved
and the field-owner defect is now resolved.

## 2026-10-31: pinned field-reader follow-up

Repair nested-value field reader facts introduced by upsert conflict bindings. Preserve settled hiding semantics and cover registration and live-migration gates. Make this bounded analysis explicitly account for kernel forms so new forms cannot silently omit facts. The nonhiding whole-probe admission criterion remains outside the authorized change unless existing contracts settle it; raise a concrete coupled decision before changing observable policy.

The two source tracks use separate files and warm workspaces. Authors own
execution and corrections; fresh independent reviews precede a lead-assigned
combined final check when both are ready. No duplicate full runs.


## 2026-11-01: field-reader correction remains unlanded

Independent review reproduced a new complexity regression in the reader
candidate: a small owner query with ten independent four-constructor ranges
exhausts the authority evaluator budget instead of registering and returning
zero. Reusing full authority products for field observation caused this cost.
A joined-domain field prototype avoids that product on four related forms,
but loses captured Case/partial-Let source correlation and falsely refuses a
valid registration. That prototype is rejected and must not land.

The author is staging a demand-sensitive shared-evaluator experiment with
explicit correlated dependencies and work counters. Authority behavior and
exact stored-versus-proposed/copy/snapshot distinctions must be preserved.
No budget increase, missing-facts fallback or query-specific workaround is
accepted. Automatic approval review rejected the first broad source rewrite
before execution; a bounded patch, exact backups and targeted controls are
being prepared for review before another execution request. This is not a
corrected candidate or a passing final check. Separate fixture migrations
have landed independently and do not wait on this production redesign.


## 2026-11-01: reader prototype stopped before integration

The separated-demand replay prototype is retired as a source candidate.
Its staged Rust scope grew to +1338/-77 lines, beyond the agreed estimate,
yet still requires exact fallback for one observer selecting among captured
producer alternatives. Shape construction also allocates before the proposed
shared work budget can stop it. No live application, build or test ran on
that prototype; formatting evidence is not execution evidence.

Keep the existing exact authority analyzer and current Reader integration
unchanged. Neither the product-cost candidate nor the imprecise joined-domain
candidate lands. The observed new owner-registration refusal belongs to the
unlanded integration, not a newly accepted refusal policy. The nested reader
bug and nonhiding admission question remain open.

Cause: unguarded value joins lose the producer/control correlations needed
for precise field observations, while exact constructor alternatives multiply
independent domains. This intersects the unbounded-work class. No class-wide
mechanism is delivered. A read-only architecture assessment found no material
simplification of replay that preserves attribution and coverage. Guarded
native provenance is a possible future direction, with unresolved identity,
guard-algebra and allocation/copy contracts; it is not an implementation plan.
Preserve the stopped experiments and their existing executed controls.
Further source work requires a bounded private Value contract covering capture,
independent aliases, selected producers and budget enforcement before allocation,
then a concrete reviewed implementation scope. No new public semantics or spec
ruling is made. Separate fixture and benchmark checkpoints remain delivered.

## 2026-11-01: correlation and budget contract assigned

The active user goal now includes resolving the private correlation and budget
contract, alongside completing benchmark reporting. Workspace
`pinned-reader-contract`, based on `9c4de3b56b68`, owns the contract assessment
and a bounded discriminating executable model where useful. It does not own an
engine rewrite or a change to registration/admission policy.

Acceptance requires an implementable contract for captured Case and partial Let,
independent aliases of one relation, and selection among captured producers;
explicit guard identity/algebra and value join/projection/copy laws; preservation
of stored, proposed, copied and snapshot authority distinctions; and an inventory
of allocation/copy/environment boundaries with budget enforcement before the
covered allocation. Exact refusal on budget exhaustion must not become missing
reader facts. Compare the strongest alternative on the same cases, state actual
growth bounds, and use independent expected facts and mutation-sensitive bounded
model searches where feasible. A model does not certify native integration.

A fresh independent implementability review and lead ruling must resolve the
contract before further source implementation. Any required observable semantic
change returns to the user-owned design space. Existing authority evaluation and
Reader integration stay unchanged; retired prototypes stay unlanded. Benchmark
reporting has separate ownership and verification and continues in parallel.

## 2026-11-01: private correlation and budget contract resolved

The accepted internal contract landed as `74c3ed967abc` in
[`PINNED_VALUE_CONTRACT.md`](../../crates/adtb-lang/src/permission/PINNED_VALUE_CONTRACT.md).
Use component-factored finite-domain decision diagrams for reader facts, with
distinct draw, invocation, copy and snapshot identities. Native For return joins
away its local draw; still-live outer capture conditions remain. Later ranges
draw from the joined source. Native authority evaluation and public admission
policy remain unchanged.

Independent implementability review rejected an initial template contract because
it preserved correlations native join discards. The focused correction removes
templates and specifies local closing, all-contributor witness retention, dynamic
fresh-event identity, semantic dependency closure and shared decode/native
continuation/import. Built-in aggregate observations remain componentwise. All
analysis-owned visits and requested capacity must be charged before construction;
exhaustion is explicit, never an empty field result. Physical allocator residency
and process RSS are outside this accounting contract.

The corrected source-guided miniature passed 224 decoded comparisons, including
same-constructor field joins, preserved outer captures, close/rebind, witnesses
and selected producers. Retaining closed correlation and erasing live correlation
both change expected results. These are bounded algebra observations, not native
registration or permission-parity evidence. The accepted contract records the
algorithms, concrete distinguishing examples, allocation inventory and limits.

This resolves the requested contract prerequisite, not the reader bug. Next:
estimate a bounded implementation at the existing Value/evaluation seams, retain
the exact authority path, and verify native parity, owner Count registration,
capture/alias/selection controls and allocation sealing before activating Reader
integration. Nonhiding admission remains a separate unresolved question. No
retired prototype landed and no defect class is declared closed. This ticket
returns to open because the contract track has finished.
