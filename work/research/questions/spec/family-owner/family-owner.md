---
title: "Who may register into a bundle family"
pick: one
options:
  first-registrant:
    title: "The first registrant owns the family; only it or the schema owner registers into it; family call grants compare signatures by type ids"
  anyone:
    title: "Any role able to register"
---
Raised by the third review of the spec in docs/spec/.

## spec/family-owner/first-registrant

The first registrant owns the family; only it or the schema owner registers into it; family call grants compare signatures by type ids. A family call grant otherwise extends to code any registering role ships.

## spec/family-owner/anyone

Any role able to register. A grantee of web::f can be handed another role's code.
