---
created-at: 2026-11-07
status: open
kind: feature
priority: 15
summary: "write::Database is the whole engine under one name, Txn has no Drop so begin is paired by hand at 10 sites, lock preconditions are comments, and errors cross four layers by hand and as text"
---

# Feature: Transactions and errors with owners in the write path

## Request

## Proposal

## Open questions

## 2026-11-07

Audit: work/briefs/audit-structure-r1-exec.md (local). api/verify.rs:459-462 returns through `?` after `begin` without aborting. 16 error enums, 20 `From` impls, 40 `{e:?}` sites; `ends_transaction` a hand-kept list. Executor inputs are pub-field structs filled per site; SSI read recording chosen per site; execution.md §1.3's 9 plan invariants checked nowhere, 30 `Broken` checks at run time instead.

Exit: a transaction guard that aborts on drop; lock preconditions as guard parameters; errors typed through to the api with no `{e:?}` conversion; executor inputs built by one constructor each; a plan validator for execution.md §1.3.

## 2026-11-08, from the version-owner design

The scoped snapshot: a registered reader's snapshot holds back only its plan's heaps, and nothing checks that it reads only those (`Scope::Heaps`); a wrong scope lets vacuum free a version the reader then walks. Lead ruling (structure-version-owner Q7): the `Reader` carries the `Scope` it was taken with and `Store` checks each heap against it, in this program, after structure-version-owner step 2 adds the `Reader`.
