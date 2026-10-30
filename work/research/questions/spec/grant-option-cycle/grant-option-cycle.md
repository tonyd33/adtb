---
title: "Whether grant options may form a cycle"
pick: one
options:
  refuse:
    title: "A grant with grant option that would close a cycle is refused"
  reachable:
    title: "Cycles are allowed; an orphaned row is one no chain from the schema owner supports"
---
Raised by the grant-rev30 review. x gives a `monitor` with option, a gives b with option, b gives a with option, x revokes a. Each row relies on another, so every revoke is refused and the access stays. Only `drop_role` clears it, and that is refused for a role the log mentions.

## spec/grant-option-cycle/refuse

The option graph stays acyclic, so a revoke of the leaves first always succeeds. The refused grantee loses nothing: it gives the grantee an option through a chain that already rests on the grantee.

## spec/grant-option-cycle/reachable

SQL's model. It still refuses each single revoke of a cycle unless all go in one step, and adds graph reachability to every revoke, for grants nobody needs.
