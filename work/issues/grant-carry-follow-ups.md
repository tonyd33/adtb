---
created-at: 2026-10-29
status: open
kind: bug
priority: 1031
summary: "grant carry leaves fieldless and redundant grant entries, accepts no-op retags, matches listed revokes only exactly, and security.md 3.3 and 3.6 do not state four of its rulings"
tags:
  - lang
---

# Bug: grant carry follow-ups

## Report

From the five reviews of grant-carry (work/briefs/review-grant-carry{,2,3,4,5}.md, local). Rulings are in grant-carry.md.

Code:
- A split by `provider` leaves `update identities.OAuth (provider)` as an `update` entry covering no field; drop it.
- A regrant after a division holds both `* -> *` and `* -> * except [..]`; drop the subsumed entry.
- A no-op `retag X -> X` is accepted, and any retag naming a merged or nested constructor clears the merge/nest refusal (only loses access).
- A listed revoke after a carry must name the carried field set exactly, and a revoke that matches nothing reports nothing (see spec-evolve-gaps for a revoke of a privilege never granted).
- A listed set grant naming a field some constructor of the set lacks is accepted, against security.md 3.3 (predates grant carry).
- A guarded fill arm counts as not carrying while a split's routing counts every field under a guard; the two rules differ (both only strict).
- `unstated` lives on `Source`, cleared in `migrate_keeping`; it belongs to the migration state.
- carry.rs: seed the exception set with each source's self pair and let `excepted` map it, removing the second loop (643-654); one helper for the two `Unstated` pushes (619-623, 636-641).
- Dump: state alone cannot print an unlisted whole-relation grant whose relation later gained constructors, or an excepted retag; a dump format must print them.

Spec (spec-revision, security.md):
- 3.3: an unlisted grant on a named set covers every field of a constructor joining the set; a listed one covers the listed fields it has (ruling 1).
- 3.6: a split product's constructor reveals its routing to a role reading it (ruling 11).
- 3.6: a merge or nest of sources granted unalike, including by separate grants or excluded retag pairs, is refused unless restated (rulings 5 and 13); `update` of a merged discriminator or nesting field needs retag between every pair of sources, and a retag between products of one source needs `update` of the routed fields (rulings 8 to 10).
- 3.6: move field up with a fill (ruling 7).
