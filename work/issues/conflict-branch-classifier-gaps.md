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

## 2026-11-02: from the misread review

- The conflict-branches.txt rows for filtered branches cannot catch a wrong `replaces`: `update old where` names `old`, so `uses_row` blocks either way. A row needs a branch that does not name `old` and ranges over another relation.
- upsert-results.txt lacks a target retag, a variable-pattern or nested-match alias, a `Par` mixing filtered and unfiltered replaces, and a set source on a filtered branch. Dropping the alias insert in `Terms::replaces` likely fails no case.
- `match (only {old}) | ...` makes `replaces` false, so `w.body` is refused though it is a replacement.
- Filtered and ranged branches with an unused result are stale at registration. Filter: the target names `old`. Ranged: cause not confirmed.

## 2026-11-02: after any branch naming old blocks

`fix: count any conflict branch naming old as reading the relation` makes the use classifier items above moot: `filter-on-new` blocks by the ruling, `kept_fields` is gone. It dropped live value checks with no replacement:

- A set source and a `rejects` result replacing a row of another constructor (rows `replaced-set`, `replaced-rejects`). Add two set source cases over a stored `Link` to crates/adtb-conformance/tests/upsert-results.txt.
- A replace beside a real second write (`replaced-beside-write`); that fixture pairs it with `skip` only.

Also: no row pins `let w = .. else skip; delete w` or `update w set ..` now blocking, `update x in notes`, `delete x in notes`, or a nested upsert. `keeping_upsert` matches only a direct upsert as the bound write, so an upsert under an `if` is unprobed. It now only separates an all `fail` branch from others, and may simplify.
