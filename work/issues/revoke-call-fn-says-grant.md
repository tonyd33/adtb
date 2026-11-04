---
created-at: 2026-11-04
status: open
kind: bug
priority: 690
summary: Refusing a deployment revoke of call fn names it a grant
---

# Revoke of call fn refused with a grant message

## Report

A deployment `revoke call fn ...` is refused with "`grant call fn` is a schema operator; write it in a migration" (crates/adtb-lang/src/evolve/grant.rs, the `CallFn` arm of the revoke path). The statement is a revoke, so the message names the wrong statement. scripts/authority/operator-grant-as-deployment-revoke.test pins the current wording and changes with the fix.

## Proposal

Name the statement given: "`revoke call fn` is a schema operator; write it in a migration".
