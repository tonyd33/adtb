---
title: "Which grants are schema and which are deployment state"
pick: one
options:
  split:
    title: "Relation, constructor-set and call fn grants are migration operators; roles, capabilities and bundle call grants are ad hoc statements; each refused in the other place"
  all-log:
    title: "Every grant is a log operator"
  all-adhoc:
    title: "Every grant is an ad hoc statement"
---
Raised by the second review of the spec in docs/spec/.

## spec/grant-kinds/split

Relation, constructor-set and call fn grants are migration operators; roles, capabilities and bundle call grants are ad hoc statements; each refused in the other place. Access to data shapes travels with the schema and its hash; who may deploy and operate is deployment state.

## spec/grant-kinds/all-log

Every grant is a log operator. Roles and passwords would enter the hashed schema.

## spec/grant-kinds/all-adhoc

Every grant is an ad hoc statement. Every grant is an ad hoc statement.
