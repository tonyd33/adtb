---
title: Relations a policy reads have policies of their own
pick: one
raised-by: [security/policy-reads/relations]
options:
  bound:
    title: Reads inside a policy run with the policy owner's rights, unfiltered
    links:
      - {rel: prior-art, to: prior/security-sqlserver-rls}
  stratified:
    title: Policies filter policy reads; the graph of policies reading relations must be acyclic
  live:
    title: Live with it
---
A policy on `docs` reading `grants`, whose policy reads `docs`, loops.
Supabase breaks such loops with security definer functions.

## security/policy-reads/recursion/bound

SQL Server `SCHEMABINDING = ON`: tables the predicate joins need no
permission for the querying user.

## security/policy-reads/recursion/stratified

Checked when a policy is declared, as `lang/recursion/negation/stratified`
checks fixpoints.

## security/policy-reads/recursion/live

Runtime recursion error.
