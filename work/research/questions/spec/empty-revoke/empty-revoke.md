---
title: "What a revoke that matches no held grant does"
pick: one
options:
  refuse:
    title: "Refuse the migration, naming the revoke"
  noop:
    title: "Change nothing, silently"
  warn:
    title: "Change nothing and list it in the migration report"
---
Raised by spec-evolve-gaps and grant-carry-follow-ups. security.md does not say
what `revoke read users to web` does when web holds no matching grant, or what a
listed revoke does when its field list matches no held field set.

## spec/empty-revoke/refuse

The migration is refused, naming the revoke and the role. A revoke states that a
role holds something and should stop; when it holds nothing matching, the author's
picture of the grants is wrong, and the likeliest cause is a typo that leaves the
intended access in place. A migration is refused as a whole, so nothing applies.

## spec/empty-revoke/noop

The revoke changes nothing. Idempotent, but a mistyped revoke leaves access in
place with no sign.

## spec/empty-revoke/warn

The migration applies and its report lists each empty revoke. Embedded opens and
scripted deploys rarely read reports, so this is close to noop in practice.
