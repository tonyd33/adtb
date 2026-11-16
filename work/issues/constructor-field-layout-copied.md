---
created-at: 2026-11-16
status: resolved
closed: 2026-11-16
kind: bug
priority: 880
summary: "matching a hand list of field names to a constructor's declared fields, and filling declared slots from it, is written three times: catalog codecs, error sums and verify's kept rows"
tags:
  - quality
  - engine
  - class-copied-rules
---

# Constructor field layout copied

## Report

Found by the quality review of reviewed..cdb9f9f5. Three places resolve a Rust list of field names against the fields a schema declares for one constructor:

- `Codec::new` (crates/adtb/src/catalog/codec.rs:238-255) sorts the listed and declared names, asserts they are equal, then finds each listed name's declared position.
- `Sums::new` (crates/adtb/src/api/error_value.rs:177-190) does the same for the prelude error, origin and outcome sums, with its own assertion text, and also counts each sum's constructors (:195, :207-214) the way `Codec::new` checks a nested sum's constructor set (codec.rs:282-289).

Three places then fill declared slots from field handles:

- `Codec::declared` (codec.rs:336-343)
- `Sums::of_ctor` (error_value.rs:583-594)
- `Kept::fields` (crates/adtb/src/api/verify.rs:197-205)

Each fills a `vec![None; n]` by handle and unwraps every slot with its own panic message.

The copies can drift. One might, for example, accept a duplicate listed name that another refuses.

Fix direction: one constructor layout in `catalog::codec`, built from a schema's declared fields and a `Fields` list, that the codec, the error sums and verify's kept rows share. It should give the position and type of each listed field and fill declared slots from field handles.

## Resolved

Landed as rknzqrry e9f2552f (catalog-encoders-through-codecs part A, 2 rounds, full check 0, 3853 tests). One layout, `codec::Listed` with `codec::listed`, serves `Codec::new`, `Codec::declared`, `Codec::with`, `Codec::encode`, `Sums::new`/`Sums::of_ctor` and verify's `Kept::fields`. The searching codec test writes every constructor of every codec under the shipped and both reversed catalogs, with a distinct value per field, against the row placed by name.
