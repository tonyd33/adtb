---
title: How UUID presentation covers values outside its write-time domain
pick: one
options:
  lossless:
    title: Use generic newtype text outside the sixteen-byte UUID domain
    needs: [spec/uuid/uuid-newtype, constraint/refine/write-only]
  intrinsic:
    title: Replace the Bytes newtype with an intrinsic fixed-size UUID type
    excludes: [spec/uuid/uuid-newtype]
  fault:
    title: Make UUID formatting fallible for other byte lengths
    needs: [spec/uuid/uuid-newtype, constraint/refine/write-only]
links:
  - {rel: resolved-by, to: uuid-domain-formatting}
---
Uuid is a Bytes newtype with a write-only length predicate. Consequently a
one-byte Uuid is a legitimate in-memory value, but current text and print reject
it as an internal error. Prelude2.7 promises total text with fixed UUID notation;
interop1.1 and design/kernel9 promise every-value print/parse. The contracts need
a choice for values outside the storage domain. Evidence and source references
are in notes/uuid-domain-formatting.md.

## spec/uuid-domain-format/lossless

Preserve Uuid as a newtype and its relation-write length check. For16bytes print
uuid("61626364-6566-6768-696a-6b6c6d6e6f70") and text the bare lowercase
hyphenated form, as today. For any other length print and text use the generic
newtype spelling Uuid(0x...), including Uuid(0x) for empty bytes. Uuid.parse reads
both wrappers at the known type, without evaluating the write predicate. Printing
chooses the UUID wrapper whenever length is16, so equal values print identically.
The bare Text accepted by uuid(t) remains strict fixed UUID syntax. No bytes are
padded or truncated. Generic newtype and Bytes parsing already provide the
needed mechanisms, but the special Uuid reader must admit the generic wrapper.

The maintainer recommends this because it preserves construction/domain timing,
existing stored UUID presentation, total text and universal roundtrip. The cost
is documenting a second spelling for in-memory values outside the domain.

## spec/uuid-domain-format/intrinsic

Make Uuid an intrinsic value type whose values always contain16bytes. Replace
rather than silently strengthen its existing newtype definition; define a checked
Bytes conversion and migration/API consequences. This requires reopening the
adopted UUID-newtype choice. It restores fixed-size formatting but changes the
model and constructor contract for all callers.

## spec/uuid-domain-format/fault

Retain all Bytes as constructible Uuid values, but make text return an explicit
fault or checked result for lengths other than16. Canonical print still needs a
lossless fallback or a weaker every-value guarantee; choosing a fault throughout
would also revise interop1.1 and design/kernel9. This adds a special partial
presentation API to a generic newtype. The maintainer does not recommend weakening
those guarantees to preserve a formatter assumption.
