---
created-at: 2026-10-28
status: open
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

Workspace /home/tony/code/adtb-uuid-audit; local brief work/briefs/brief-uuid-audit.md. Preserve baseline and failed evidence; receipt must name exact tested inputs, statuses and unresolved checks. No dependencies added.

## 2026-10-28: executable evidence and proposed resolution

Fresh evidence-only statea1a1e3d261e9f303d9c68245a97a384ff44448bf on verified
main5debace3ea0f97765bc72282bbb0d721b8cfc851 confirmed the problem in engine AND
reference. Uuid(bytes("x")) and unwrap succeed with Bytes([120]); text and print
return Internal("a `Uuid` is 16 bytes, found 1"). Printing the successful raw
query result separately gives PrintError. Sixteen-byte controls succeed.

From /home/tony/code/adtb-uuid-audit run:

```
direnv exec /home/tony/code/adtb env DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target cargo nextest run -p adtb-sim --test uuid_contract_evidence --success-output immediate --failure-output immediate
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
