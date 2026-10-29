---
title: The guard and the body bind the pattern's names at two sites
pick: one
raised-by: [lang/patterns/failure-sharing/retest]
options:
  second-scope:
    title: Check the guard under its own binder variables
  share-var:
    title: Bind the same variable at both sites and amend the kernel contract
---
Today the guard and the body are checked in one scope
(pat.rs `arm_bodies`, :1002-1024) under the arm's binder variables. With
`retest` the guard sits in the condition and the body under the second
destructuring, and docs/design/kernel.md §2 says no two binding sites
share a variable.

## lang/patterns/failure-sharing/guard-vars/second-scope

`arm_patterns` creates a second variable per name, same type;
`arm_bodies` checks the guard in a scope binding the names to those.
No term is copied.

## lang/patterns/failure-sharing/guard-vars/share-var

Both sites bind the arm's variable. Or-pattern alternatives already bind
one variable at several exclusive sites (pat.rs:1116 under the loop at
:1123-1128), so §2 would state the exception for exclusive sites.
