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
