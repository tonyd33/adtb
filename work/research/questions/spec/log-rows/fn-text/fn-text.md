---
title: "What a functions row's signature and body hold"
pick: one
options:
  as-written:
    title: "As written; an aggregate's body is everything after ="
  canonical:
    title: "In canonical token form"
---
Raised while writing the migration log rows. `functions.signature`
and `body` did not say whether they are the text as written or a
canonical form, nor where an aggregate's body starts.

## spec/log-rows/fn-text/as-written

`signature` and `body` are the declaration's text as written. An
aggregate's body is everything after the `=` of its declaration.

## spec/log-rows/fn-text/canonical

Canonical token form. The row then differs from what the author wrote,
and the canonical form is already what the hash covers.
