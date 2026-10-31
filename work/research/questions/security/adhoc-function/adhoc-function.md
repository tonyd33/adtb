---
title: What ad hoc text needs to call a schema function
pick: one
options:
  call-and-reads:
    title: The role's call fn grant, and its body's reads charged to the role
  reads-only:
    title: The body's reads only
---
`pure_secret()` returning a literal reads nothing, so a role with no grant gets the literal.

## security/adhoc-function/call-and-reads

Calling a function is a privilege, as 3.1 says; text gains no definer rights.

## security/adhoc-function/reads-only

A function's literals and existence go to any adhoc role.
