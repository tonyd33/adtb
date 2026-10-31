---
created-at: 2026-10-31
status: in-progress
kind: bug
priority: 399
summary: "A pinned bundle's write reaches constructors past its pin: Internal on a legal upsert, a hidden row overwritten, a hidden value-sum field misread"
branch: pinned-probes
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
