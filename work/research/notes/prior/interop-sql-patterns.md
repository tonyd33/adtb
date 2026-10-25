---
title: SQL encodings of sums in the wild
---
Rails single table inheritance (one table, a type column naming the subclass) and polymorphic belongs_to (a type column plus an id, no FK possible). Postgres table inheritance does not carry unique, primary key or FK constraints to children.
https://guides.rubyonrails.org/association_basics.html
https://www.postgresql.org/docs/current/ddl-inherit.html
