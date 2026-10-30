---
title: "How a migration removes only a grant option"
pick: one
options:
  replace:
    title: "Revoke, then grant without the option, in one migration; 3.5 says so"
  form:
    title: "A new form, revoke grant option for P on T from R"
---
Raised by the checked-registration track. security.md 3.5 speaks of "a revoke
that removes a grant option", but the grammar has no revoke of the option alone,
and a regrant without `with grant option` keeps the option held.

## spec/option-revoke/replace

Removing only the option is `revoke` followed by the same `grant` without
`with grant option`, in one migration. 3.5 is reworded to "a migration that
removes a grant option some live bundle's registration relied on". No new form;
the recheck already sees the migration's net effect.

## spec/option-revoke/form

Add `revoke grant option for`. One statement instead of two, at the cost of a new
grammar form, a new logged operator and its inverse.
