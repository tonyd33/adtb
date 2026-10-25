---
title: Some registered queries have no translation across an operator
pick: any
raised-by: [surface/version-serving/translate]
options:
  translate-at-apply:
    title: Every registered query is translated when the operator applies; an untranslatable one refuses it
    needs: [surface/registration/on-change/refuse]
  runtime-error:
    title: Untranslatable calls fail when made
  live:
    title: Live with whatever translation does
    excludes: [surface/version-serving/untranslatable/translate-at-apply, surface/version-serving/untranslatable/runtime-error]
---
Translation through inverses is exact for rename, split (through its
routing match), merge (through its discriminator), nest and reads of
hoisted fields. It is not for a write, at an old version, to a
constructor removed since, and a translated write may break a
constraint added later.

## surface/version-serving/untranslatable/translate-at-apply

```
$ adt migrate 0014   -- remove constructor Identity.Facebook
refused: bundle ios-4.2 pins 12: query link_facebook inserts identities.Facebook
retire bundle ios-4.2 or remove link_facebook from it first
```

Translation happens once, when the operator is applied, so a refusal
names the query. A translated write that violates a constraint added
later fails as an ordinary constraint error at call time; that is the
constraint working, not a translation gap.

## surface/version-serving/untranslatable/runtime-error

The operator applies; the call fails later.

## surface/version-serving/untranslatable/live

Unspecified.
