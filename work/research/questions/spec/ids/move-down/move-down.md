---
title: "Which listed constructor keeps a header field's id on a move down"
pick: one
raised-by: [spec/ids/moved-field/keeps]
options:
  first-declared:
    title: "The listed constructor first in the type's declaration order; the others take new ids in that order"
  first-listed:
    title: "The constructor listed first in the operator; the others take new ids in listed order"
---
Raised by the catalog id work. `move field T.f to T.(C | D)` makes one
field per listed constructor, and one of them keeps the header field's
id.

## spec/ids/move-down/first-declared

The field of the listed constructor that comes first in the type's
declaration order keeps the id, and the other listed constructors'
fields take new ids in declaration order. `to T.(Google | EmailPass)`
and `to T.(EmailPass | Google)` give the same ids.

## spec/ids/move-down/first-listed

The first constructor written in the operator keeps the id. Two
spellings of one move give different ids.
