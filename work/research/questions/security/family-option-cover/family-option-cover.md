---
title: What a family row with grant option lets its holder give
pick: one
options:
  equal-ids-now:
    title: It covers a bundle's query exactly while their ids are equal; a regrant that uncovers a given row is refused
  by-name:
    title: It covers the query by name whatever its signature
---
```adt
-- a holds call web::q at 41 sig1 with grant option, and gave b web-2.4::q
grant call web::q at 45 sig2 to a with grant option  -- the owner regrants
-- or a gives web-2.5::q, whose signature differs from sig1
```

## security/family-option-cover/equal-ids-now

Coverage for giving is the coverage for calling: equal ids at that moment. Giving outside it is refused. A regrant that would leave a row given under it uncovered is refused naming the row, as a revoke that orphans it is.

## security/family-option-cover/by-name

Lets a holder give what it could not itself call.
