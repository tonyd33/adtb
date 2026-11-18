---
created-at: 2026-11-17
status: open
kind: bug
priority: 700
summary: a migration whose index element is refused also reports no relation in scope for a Key field of the same type after the real refusal
tags:
  - lang
  - class-none
---

# Refused element cascades scope error

## Report

Found by index-keyable (2026-11-17). A migration whose index element is refused, over a type that declares `id: Key items`, also reports "no `items` in scope" after the real refusal; main does the same with `index (zz)`. Expected: the refusal of the element alone, with no follow-on error from the relation the refused declaration left undefined.
