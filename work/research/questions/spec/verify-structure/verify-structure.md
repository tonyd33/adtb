---
title: "What a verify finding says for damage outside the named constructors"
pick: one
options:
  structure:
    title: "A Structure constructor with a location text"
  order:
    title: "Fold it into Order"
---
Raised by the milestone 9 audit. Verification finds a leaked or doubly used page and a heap directory that its own check refuses; `Finding` has no constructor for damage that is not an index's and not a page's.

## spec/verify-structure/structure

`Structure { where: Text }` holds any structural failure outside a checksum, a stale page or an index order.

## spec/verify-structure/order

`Order` names an index, so a heap's damage has nothing to name.
