---
title: Which constraint classes are declarable beyond keys and FKs
pick: any
options:
  row-check:
    title: Checks over one row's fields
    links:
      - {rel: same-as, to: surface/constraint/check}
      - {rel: prior-art, to: prior/constraint-pg-exclude}
  domain:
    title: Predicates on a field's type, independent of the row
    links:
      - {rel: prior-art, to: prior/constraint-liquid}
      - {rel: prior-art, to: prior/constraint-typedb}
  fd:
    title: Functional dependencies not implied by a key
    links:
      - {rel: prior-art, to: prior/constraint-logiql}
  exclusion:
    title: No two rows of a scope stand in a given relation
    links:
      - {rel: prior-art, to: prior/constraint-pg-exclude}
  inclusion:
    title: General inclusion dependencies between two queries
    links:
      - {rel: variant-of, to: constraint/at-least-one}
  cardinality:
    title: Bounds on how many related rows each row has
    links:
      - {rel: prior-art, to: prior/constraint-typedb}
  assertion:
    title: Arbitrary database-wide assertions over any query
    links:
      - {rel: prior-art, to: prior/constraint-tutorial-d}
      - {rel: prior-art, to: prior/constraint-sql-assertion}
      - {rel: prior-art, to: prior/constraint-alloy}
      - {rel: prior-art, to: prior/constraint-logiql}
      - {rel: prior-art, to: prior/datomic}
---
Keys over constructor sets, FKs to constructor sets and at-least-one
participation are settled. These are the further classes. They stack;
with `assertion`, every other class is expressible, and the rest are
the shapes the engine checks with a dedicated mechanism instead of a
general one.

## constraint/row-check

```
check email_length on identities.EmailPass (length(email) <= 254)
```

Reads only the row written. Postgres `CHECK`, Gel `constraint
expression on`, which refuses paths of more than one hop.

## constraint/domain

```
type Email = Text where matches(self, "^[^@]+@[^@]+$")
type Identity = EmailPass { email: Email, hash: Bytes } | Google { gid: Text }
```

SQL `CREATE DOMAIN ... CHECK`, Tutorial D possrep constraints, TypeDB
`@range`, `@regex`, `@values`.

## constraint/fd

```
constraint zip_state =
  from a in addresses.US, b in addresses.US
  where a.zip == b.zip and a.state != b.state
  select { zip: a.zip }
```

LogiQL writes FDs as right-arrow constraints. A key is the FD whose
right side is every field; a non-key FD is a denormalization the engine
keeps honest.

## constraint/exclusion

```
constraint no_double_booking =
  from a in bookings, b in bookings
  where a.room == b.room and overlaps(a.during, b.during) and key a != key b
  select { a: key a, b: key b }
```

Postgres `EXCLUDE USING gist (room WITH =, during WITH &&)`; Postgres 18
`WITHOUT OVERLAPS` is the equality-plus-one-range case. Unique is
exclusion under `==` on every field.

## constraint/inclusion

```
constraint assigned_member =
  from a in assignments
  where count(m for m in memberships where m.user == a.user and m.org == a.org) == 0
  select a
```

Any projection of one query inside another. An FK is the case whose
right side is a key; `constraint/at-least-one` is the case from parent
into child.

## constraint/cardinality

```
constraint session_limit =
  from u in users
  where count(s for s in sessions where s.user == key u) > 5
  select u
```

Pairs with at-least-one. TypeDB `@card(n..m)`, EER participation
bounds.

## constraint/assertion

```
constraint balanced =
  from e in entries
  where sum(l.amount for l in lines where l.entry == key e) != 0
  select e
```

Any query over the database. Tutorial D `CONSTRAINT` over any relational
expression; SQL-92 `CREATE ASSERTION`, first shipped by Oracle 26ai in
2026 with aggregates, outer joins and set operators refused; Alloy
`fact`; Datomic `:db.entity/preds` run on demand only.
