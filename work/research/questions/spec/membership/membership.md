---
title: "Which constructor-set membership a pinned reader sees"
pick: one
options:
  pinned:
    title: "Membership at the pinned version; enabling C blocks on readers of the whole relation or of references admitting C"
  current:
    title: "Current membership"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/membership/pinned

Membership at the pinned version; enabling C blocks on readers of the whole relation or of references admitting C. Old readers keep exhaustive matches and never dereference into an unknown constructor.

## spec/membership/current

Current membership. Old readers would meet constructors they cannot type.
