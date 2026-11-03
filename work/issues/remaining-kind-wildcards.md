---
created-at: 2026-11-02
status: open
kind: bug
priority: 130
summary: "Other Kind wildcards in translation, single form wildcards and Prim operators are not named"
---

# Remaining kind wildcards

## Report

The slice in `fix: deny wildcard arms over Kind in reader and translation analyses` found a wrong answer behind `generic`'s `Restrict` arm, so the unjudged wildcards deserve the same reading.

- crates/adtb-lang/src/translate/walk.rs: `kind`, `stored_whole`, `reads_probed`, `probed_rows`, `rows_of`, `yields`, and those at lines 67-129. `kind` and `stored_whole` delegate to `generic` and `carries`.
- `clippy::wildcard_enum_match_arm` does not fire on a wildcard covering exactly one form; `clippy::match_wildcard_for_single_variants` does and is not denied.
- `Kind::Prim(..)` arms in `compared` and `reads_of` do not name operators.
- Nothing at type level ties a constructor set to its mapping in `generic`.
- A `match` on a selection of a removed constructor builds a zero arm `Case`; `a_pinned_selection_reads_its_constructors_after_a_removal_moves_their_positions` reads only `x.n`. Add `match`.

## 2026-11-02: judged

`fix: name every Kind in the remaining translation analyses` names every form in `mark`, `stored_whole`, `kind`, `reads_probed`, `probed_rows`, `rows_of`, `yields`, and denies both lints there and on the earlier analyses. It fixed a wrong answer: after `remove constructor T.A`, a pinned `query q () -> Tag T = B` returned `C`. A `match` on a removed constructor's selection is right. Full check exit 0; review found no blocker and no other position kept in translate, by tracing.

Left: `Prim` operators; a type tying a constructor set to its mapping; `_` over `Kind` at walk.rs `case` neighbours and over `Ty` in `case`, correct today, with no deny.

## 2026-11-03: Prim operators landed

`fix: name every Prim in the translation analyses` names every `Prim` in `compared` under the deny of both lints. It fixed a wrong answer: a pinned `print` or `T.parse` of a value of a user type answered with the later version's names after a rename of a constructor, field, type, record type or relation, or an added field; a key printed or parsed by its relation's later name, a renamed key field or a renamed newtype in a key field. Such a migration is now refused naming the operator (`Map::shown`, `Map::key_shown`, `blame_rename`); evolution.md 7.1 asks for the pinned answer instead, carried on pinned-print-parse-translation. Other Prims judged by reading: values pass through unchanged. Search tests `a_pinned_print_or_parse_answers_as_at_its_pin_or_is_refused_naming_the_operator_across_a_change_of_a_sum` and `..._across_a_rename_elsewhere`: 10 bodies by 8 operators, 66 translated and 14 refused, all reached. Full check 3628 passed. Review found a composite key permuted by sorted position across a rename, carried on pinned-composite-key-order.

Left: `Kind::Prim(..)` arms unnamed in the structural analyses (`mark`, `stored_whole`, `kind`'s generic arm, `generic`, `carries`, forms.rs), where no Prim holds rows, probes or a carried branch; `_` arms with no deny over `Kind` in `field` and `replace`, over `Ty` in `case`, over `Option<Site>` in `tm`, each correct today by tracing; latent over-refusal: `shown` passes `ordered = true`, so a constructor reorder would refuse a scalar print, though no operator reaches it; a type tying a constructor set to its mapping in `generic`.
