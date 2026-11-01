---
created-at: 2026-10-31
status: resolved
kind: feature
priority: 400
summary: checker cases are compiled Rust wrappers instead of runtime-loaded input and expectation data
closed: 2026-10-31
---

# Feature: Checker runtime fixtures

## Request

## Proposal

## Open questions


## 2026-10-31: bounded B108 migration

Move the 108 mapped checker acceptance/refusal cases from Rust wrappers into
runtime data. Retain one shared driver and the checked/normalized binding-site,
phase, inferred-type and external-parameter assertions each case needs. The
three identified structural/resource exceptions and adjacent tests remain.
No production compiler, dependency or normative spec change is authorized.

Require complete case/subcase membership, execution of every discovered input,
independently reviewed exact expectations, no Rust rebuild on data-only edits
or additions, measured edit feedback/runtime tradeoffs, named mutation
sensitivity and exact restoration. Targeted checks precede independent review
and one final full check. This is parallel source work while the smaller
reference-fixture pilot finishes; builds wait for that check's resource slot.

## 2026-10-31: combined final verification plan

Both implementation tracks have completed targeted verification and have
separate fresh independent reviews. After review convergence, stack the
pinned-field fix before the checker migration on current main. The checker
author owns one authoritative full check of that combined final tree; each
author retains diagnosis and corrections for their own change. Native
landing preserves both commits and reuses the receipt only after comparing
the landed tree with the tested combined tree. Do not run a separate full
check merely because each change has its own ticket.

## 2026-10-31: migration landed

Landed `bad143fb717a` with 108 families and 396 runtime subcases, including
162 exact refusals and nine inferred types. Checked and normalized root
identity assertions, parameter descriptors, extra schema and all original
input bytes are preserved. Fresh independent review reconstructed every
source operation; all 11 corrected diagnostic predictions received producer
review. Six named mutation assertions and data-only discovery/failure
controls passed with exact source restoration.

Three matched Rust edits took 4.665, 4.698 and 4.687 seconds versus runtime
data edits of 0.063, 0.116 and 0.084 seconds, with executable bytes and
mtime unchanged. Equivalent serial group costs were 0.590/0.580/0.524 seconds
before and 0.489/0.495/0.490 after. These are local observations, not
clean-build or whole-suite speedup claims.

One final combined check on `047020687851` covered this migration and the
pinned-field fix: 3650 root tests, 48 campaign tests, 13 Loom tests and
39 Miri tests passed, with existing configured skips unchanged. Every
configured lint, format, dependency, size, documentation, grammar and tool
check passed. Own exit was zero; revision, status and all 3094 tracked
inputs were unchanged. Landing added only reviewed ticket/index prose,
so this receipt was reused without another full run.

## 2026-10-31: resolved
