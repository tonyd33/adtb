---
title: "Which id a field takes when move field moves it"
pick: one
options:
  keeps:
    title: "The moved field keeps its id; a move down gives each further constructor a new field"
  new:
    title: "Each field after the move is new; the old one is dropped"
---
Raised by `id-assignment-gaps`. §1.2's table has no row for `move
field`. A move up (`move field T.C.f to T`) makes one header field of
one constructor field; a move down (`move field T.f to T.(C | D)`)
makes one field per listed constructor.

## spec/ids/moved-field/keeps

A move up keeps the field's id. A move down keeps it for the first
listed constructor, and each other listed constructor's field takes
the next field id, in listed order. Grants and statistics on the field
follow it.

## spec/ids/moved-field/new

Every field the move produces takes the next field id, and the old
field's row is marked dropped. Simple to state; grants and statistics
keyed by field id restart.
