---
created-at: 2026-10-27
status: open
kind: feature
priority: 880
summary: "several refusals name the wrong cause, a multi-item error has no item number, and an NDJSON error prints debug text"
tags:
  - lang
  - engine
---

# Feature: refusal message follow-ups

## Report

Found while writing the manual:

- `remove constructor Pet.Bird`, when a function still matches on `Bird`, says "`Bird` is not a constructor of Pet". The cause is the function's recheck; the refusal should name the function.
- `update ... set Rabbit { name: p.name, ... }` says "`name` is a key field ... and is not given". The key is given; the rule is that a retag must not give it.
- An error from an ad hoc text of several items does not name the failing item's ordinal, which `syntax.md` section 3.1 and `transactions.md` section 2 require.
- An NDJSON read error prints Rust debug text: "member `pages`: Base(I64) is a string in its textual syntax".
- In a merge's routing, `size: Size.Small` is refused while `Small` works.
- `read_csv("loans-*.csv", T)` matches nothing, while `"./loans-*.csv"` works.
- `Uuid.parse` returns None for a plain UUID text and accepts only the printed `uuid("...")` form.
