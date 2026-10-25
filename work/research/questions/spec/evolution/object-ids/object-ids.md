---
title: "Which schema objects have catalog ids"
pick: one
options:
  every-object:
    title: "Every schema object, computed fields, defaults and stored views included"
  named-only:
    title: "Only types, constructors, relation variables and indexes"
---
Raised by the follow-up review of the evolution rulings. `Operator`
names objects by catalog id, but some objects an operator touches
(computed fields, defaults, stored views) had no id.

## spec/evolution/object-ids/every-object

Every schema object has a catalog id, computed fields, defaults and
stored views included. `Operator` names every object by id.

## spec/evolution/object-ids/named-only

Only types, constructors, relation variables and indexes get ids;
other objects are named by path. A rename then changes how an
operator names them, and inverses by id break for those objects.
