---
created-at: 2026-11-02
status: open
kind: bug
priority: 500
summary: "Conflict branch use of old: over-refusals, spec list and test gaps"
---

# Conflict branch classifier gaps

## Report

## Report

From the independent review of `fix: count a conflict branch using old as reading the relation`. None gives a wrong answer.

- crates/adtb/src/api/reads.rs: a branch using `old` under a unique scoped to `Plain` marks the whole relation, though `old` stays in that scope. A bundle that worked before is refused. Apply `covers` to `old`'s constructor set, after checking whether a unique's scope can grow.
- `update old where new.body == "new" set new` blocks though the filter does not read `old`. Fixture row `filter-on-new` pins it. Classify by the filter's use of `old`.
- crates/adtb-lang/src/kernel.rs `copy_renamed` carries `transport_fields` and not `kept_fields`. Latent, a false refusal only.
- pinned-readers/conflict-branches.txt has no free row for `if` over free writes, `fail`, an upsert inside a bundle `fn`, a kept non-key header field, or `let` around a branch.
- No test calls a nonhiding branch using `old` that rewrites a `Plain` row, at the pin, since the hiding half alone keeps `"a!"`.

## 2026-11-02: from the result binding review

- `unless conflict on (slug) in Plain else skip` with a used result blocks `enable r.C` though the scope excludes `C`.
- conflict-branches.txt has no row binding a filtered or ranged branch, so nothing fails if `replaces` treats a non-target `Replace` as replacing.
- The sweep selects default live rows by the name suffix `-returned`; an explicit column would state it.
- The 7.3 parenthetical in evolution.md does not name a bound result.
