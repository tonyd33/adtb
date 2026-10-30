---
created-at: 2026-10-28
status: resolved
closed: 2026-10-28
kind: bug
priority: 799
summary: UUID formatting rejects in-memory values that only violate a write-time predicate
tags:
  - engine
  - lang
  - quality
  - spec
---

# Uuid domain textual roundtrip

## Report

Independent source review found a separate formatting inconsistency while
checking principal validation. Uuid is a Bytes newtype whose16-byte predicate is
write-only (constraints2.3, prelude2.7/3.5 and adopted constraint/refine/write-only).
The generic newtype_call in crates/adtb-lang/src/check/expr.rs2751-2785 admits a
Bytes representation without evaluating its predicate. However
crates/adtb-lang/src/textual/print.rs83-86 rejects Bytes length other than16 for
Form::Uuid. Prelude2.7 promises text(Uuid) without a fault; kernel9 promises a
print/parse roundtrip for every value. These contracts conflict for valid
in-memory values outside the write-time predicate.

Candidate public programs for an executable witness:

```adt
unwrap(Uuid(bytes("x")))
```

```adt
text(Uuid(bytes("x")))
```

The first should expose the one-byte representation; the second should not
reject a valid in-memory value. The exact error/output is NOT yet observed;
this ticket records source inference, not a completed regression. Query-source
0x01 is not a Bytes literal and must not be used as a supposed witness.

First reproduce against exact main and audit existing conformance/canonical
syntax clauses, then fix or route genuine contradictory observable formatting
through a proposed design point. Do not enforce the write-only predicate during
assertion typing or silently change constructor/domain timing. Current
asserted-principal-value-domain remains focused on shared value validation and
will not claim this formatting issue fixed. This is distinct from the existing
Uuid.parse string-syntax symptom in refusal-message-follow-ups and the mistyped
printer cases in textual-follow-ups. Local audit report: work/briefs/
review-principal-validation-draft.md; no own executable check by reviewer.

## 2026-10-28: resumed engine-first track

Evidence-only exact-main5debace3 reproduction and contract audit. Preserve one-byte and valid16-byte unwrap/text/print/parse outcomes, current canonical grammar and write-time domain distinction. No production/spec change or ruling. Compilation waits for lookup build slot. Lead decides any proposed point after executable evidence.

Workspace ../adtb-uuid-audit; local brief work/briefs/brief-uuid-audit.md. Preserve baseline and failed evidence; receipt must name exact tested inputs, statuses and unresolved checks. No dependencies added.

## 2026-10-28: executable evidence and proposed resolution

Fresh evidence-only statea1a1e3d261e9f303d9c68245a97a384ff44448bf on verified
main5debace3ea0f97765bc72282bbb0d721b8cfc851 confirmed the problem in engine AND
reference. Uuid(bytes("x")) and unwrap succeed with Bytes([120]); text and print
return Internal("a `Uuid` is 16 bytes, found 1"). Printing the successful raw
query result separately gives PrintError. Sixteen-byte controls succeed.

From ../adtb-uuid-audit run:

```
direnv exec ../adtb env DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target cargo nextest run -p adtb-sim --test uuid_contract_evidence --success-output immediate --failure-output immediate
```

Collected100,2evidence tests passed and2contract regressions failed; command
wall62.373s including cold compilation, nextest0.131s. Source/config inputs
unchanged. No production/spec change; no full check assigned for this audit.
Fixture139lines retained at that revision; target/uuid-evidence.log and
uuid-evidence-status.json preserve exact outcomes. Fixture SHA256
0b8a20a66d00a1e549f36fd57bc808a81052d8de087e5fad535c3c6e53c93e50.
Report/receipt archived locally work/briefs/report-uuid-audit.md.

Generic Uuid(0x...) grammar roundtrips lengths0,1,16,17, but actual Form::Uuid
rejects it. Existing mistyped UUID tests incorrectly equate the write predicate
with the in-memory type. Proposed point uuid-domain-format selects
spec/uuid-domain-format/lossless: current16-byte notation, lossless generic
fallback for other lengths in print AND text, paired parser support. Current
rulings unchanged; user confirmation pending. Durable evidence and alternatives
are work/research/notes/uuid-domain-formatting.md and questions/spec/
uuid-domain-format/uuid-domain-format.md. Next: only after confirmed adoption,
fresh implementation/spec owner, executable examples before spec reading review,
correct contradictory tests, regression default controls plus storage-domain
refusal, independent review and full checks. Lookup lands first. No provider
expansion while these general engine issues remain.

## 2026-10-28: confirmed design, implementation dispatched

User explicitly approved uuid-domain-format. After completing frozen Principal
integration, adopted with quandary adopt uuid-domain-format --confirmed;
quandary check0. Current accepts spec/uuid-domain-format/lossless. No other
ruling changed. Fresh workspace ../adtb-uuid-format (uuid-format),
base follows1be643fa plus this metadata, sole code/spec writer. Brief
work/briefs/brief-uuid-format.md. No new dependencies or shared signature changes.

Acceptance: common lang textual reader/printer used by both consumers; lengths
0,1,15,16,17+, total print/text, exact bytes/roundtrip,16-byte canonicalization,
strict uuid(Text), malformed/trailing parse refusals, ordinary newtypes and
stored-value domain refusal/valid16 write controls. Correct old internal tests
that classify short Bytes as mistyped. Observable contract in prelude2.7 and
interop1, cross-subsystem kernel9; newtype write rules unchanged. Run affected
lang/engine/public tests, strict changed-crate clippy/fmt/house/size;
adtb-lang spec_corpus and full conformance BEFORE fresh spec reading review.
Lead owns independent implementability review and full post-land root/app/policy/
docs check (explicit narrow branch exception). Current Principal repair lands
first and is now fully verified. Keep all previous failures and input receipts.

## 2026-10-28: resolved

Landed 1f403049e1113c59c9e08ccff95517a7dbf3d078 after the assigned checks and a
fresh independent code/spec review. All nine source files are identical to frozen
0cea33e5ae69645c266a5e5c049263ff9473dcdb; the rebase changed three ticket files.
The fix reuses generic wrapper helpers in the shared reader/printer. No consumer
special case, allocated schema/Form layer, dependency or storage rule was added.

The language tests passed (11), engine textual tests (3), public API regression
(1), executor shape test (1), and strict clippy/fmt/house/size checks. New tests
took 0.007, 0.010 and 0.160 seconds. Spec corpus (9) and all 234 conformance files
on both consumers passed before the independent reading review. The reviewer
verified nine source hashes, 24 build inputs, five absent configs and 12 log
receipts. The author's initial hash manifest covered build inputs; frozen jj
states and final source hashes identify tested source. A dedicated nested short
UUID test was not added; shared recursion was inspected and existing nested UUID
coverage remains.

The full main check passed with exit 0 on clean working copy
e27bb67674b600c29aec67bb07c7bec5269e4bfd, parent 1f403049. Command from main:

```
direnv exec ../adtb env DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target check
```

It ran from 18:04:26Z to 18:07:33Z on 2026-10-28 (187 seconds; timestamps have
one-second precision). Permission, queue and execution-only durations are unknown.
All 2,395 root tests passed, with eight skipped (53.322 seconds); all 47 campaign
tests passed, with one skipped (21.335 seconds). Lint, format, parity, dependency,
size and docs checks passed. Docs ran ten tests in 10.301 seconds and checked
420 syntax inputs, 61 pages, 6,734 links and 4,999 fragments with no failures.
All tracked inputs, local root/app Cargo configs and revision remained unchanged.

Evidence: target/uuid-format-integration/; check.log SHA256
b58765949ec812cd3d417e145bc4f56303371541b84d601994acdc5f87ee9000.
Local receipt: work/briefs/receipt-uuid-format-main.md. Historical audit failures
remain preserved. No UUID integration work remains. The separate generic-newtype
API discrepancy is tracked in textual-newtype-to-text-contract.
