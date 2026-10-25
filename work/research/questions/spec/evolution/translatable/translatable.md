---
title: "How translatability is defined"
pick: one
options:
  semantic-via-triggers:
    title: "The semantic criterion, stated through listed syntactic triggers with a defined read"
  triggers-only:
    title: "The listed triggers are the definition"
  semantic-only:
    title: "The semantic criterion alone"
---
Raised by the implementability review of the spec in docs/spec/. The
spec listed conditions under which a query has no translation and, in
semantics, a criterion on `put`, without relating them or saying what
"reads a field" means.

## spec/evolution/translatable/semantic-via-triggers

Translatability is the semantic criterion of semantics §9.6, stated
through evolution's listed syntactic triggers. A query reads field `f`
when `f` occurs in a field access or pattern, or a whole row having `f`
is returned, compared or passed to a function. Copying through `with`
into a write is not a read:

```adt
update i in identities.EmailPass set { i with email: lower(i.email) }   -- does not read hash
```

## spec/evolution/translatable/triggers-only

The list alone decides. A case the list misses is accepted and fails
at call time, which the readers-first guarantee forbids.

## spec/evolution/translatable/semantic-only

The criterion alone decides. Whether `put` is defined on every state a
write can produce is not decidable from the text, so engines would
differ on what they accept.
