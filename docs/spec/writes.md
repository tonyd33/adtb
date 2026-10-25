# Writes

This part defines what a write means: the change set it denotes, the
`insert`, `update`, `delete` and upsert forms that compute change sets
from queries, retag, generated keys at insert, and the delete actions
the engine derives from a deletion. How a change set is applied as a
transaction is in [transactions.md](transactions.md); what must hold
after it is applied is in [constraints.md](constraints.md).

Every example starts from version 12 of the evolution history: the
running schema of [README.md](README.md) after migration 0011 adds the
constructor `Apple` and 0012 enables it, so `Identity` is `EmailPass |
Google | Apple` and `identities.OAuth` is `Google | Apple`. Examples
are independent of each other. They extend that schema with:

```adt
type Login { id: Key logins, user: Key users on delete cascade }
  = Active  { identity: Key identities on delete retag Revoked {} }
  | Revoked { at: Instant }
relation logins : Login
  key id generate sequence
  default Revoked.at = generate clock

type Reset = { id: Key resets, identity: Key identities.EmailPass on delete cascade, token: Bytes }
relation resets : Reset key id generate sequence

type Badge = { id: Key badges, owner: Key users on delete restrict, label: Text }
relation badges : Badge key id generate sequence

type Import { id: Key imports } = Email { user: Key users, addr: Text, h: Bytes }
                                | Oauth { user: Key users, sub: Text }
relation imports : Import key id generate sequence

type Note = { id: Key notes, author: Option (Key users) on delete clear, body: Text }
relation notes : Note key id generate sequence

fn lookup (email: Text) -> Text = lower(email)
fn rehash (h: Bytes) -> Bytes = h
```

## 1. Change sets

### 1.1 Queries are pure

A query never changes the database. A write is an expression whose
value is a change set; nothing is changed until a transaction applies
that change set ([transactions.md §2](transactions.md#2-statements)).
The forms `insert`, `update`, `delete` and upsert are write
expressions, and so are:

- `skip`, the empty write, whose change set is empty;
- `(w1, w2, ...)`, which combines its writes into one change set as
  §1.4 combines the writes of a statement;
- `if c then w1 else w2`, and a `match` whose arms are all writes.

A write may appear only where a change set is expected: as an item of
a `transact` block, bound by `let` inside one, as an item of ad hoc
text ([transactions.md §2](transactions.md#2-statements)), in a
migration script ([evolution.md](evolution.md)), as the else branch of
an upsert, as an arm of a write `match` or `if`, as a component of a
write sequence `(w1, w2, ...)`, or parenthesised as the operand of
`key` (`key (insert users { name: n })`, §2.2). A write used as an
operand is parenthesised. A write expression inside a query, a
predicate or a function body is a type error.

`let x = w` binds rows for four forms: an insert binds the rows it
inserts (§2.2), an upsert its binding (§6.2), an update the new rows
with fills applied (§3), and a delete the rows it names, not the rows
its delete actions derive (§5). A `let` over `skip`, a write sequence,
a write `if` or `match`, or `fail` is a type error.

### 1.2 What a change set is

A change set over a database holds, for every constructor relation
`r.C`, two sets of rows of `r.C`:

- `inserted(r.C)`, the rows to add, and
- `deleted(r.C)`, the rows to remove, each a row of `r.C` in the
  pre-state.

Applying a change set to a pre-state `S` gives the state in which each
constructor relation `r.C` holds `(S(r.C) minus deleted(r.C)) union
inserted(r.C)`. Every constructor relation of every relation variable
is assigned at once: applying a change set is one multiple assignment.
There is no intermediate state in which some constructor relations
have been assigned and others have not.

Every write form below is defined by the change set it denotes:

| Form | Denotes |
| --- | --- |
| insert of row `v` into `r.C` | `v` in `inserted(r.C)` |
| delete of row `x` of `r.C` | `x` in `deleted(r.C)` |
| update of row `x` of `r.C` to `v` of `r.C` | `x` in `deleted(r.C)`, `v` in `inserted(r.C)` |
| retag of row `x` of `r.C` to `v` of `r.D` | `x` in `deleted(r.C)`, `v` in `inserted(r.D)`, with one key |

A row an update leaves equal to its old value appears in neither set.
A change set in which `inserted(r.C)` and `deleted(r.C)` share a row is
normalised by removing that row from both, so an unchanged row is never
part of a change set. Constraints that read the change
([constraints.md §6](constraints.md#6-transition-constraints)) and the
change stream ([interop.md](interop.md)) see the normalised change set.

### 1.3 Evaluation against one snapshot

Every write of a statement is computed against the same pre-state: the
snapshot of the statement. A write does not see the rows another write
of the same statement inserts or deletes, except through a name bound
by `let` ([transactions.md §3](transactions.md#3-transact-blocks)).
The order in which writes of a statement are written has no effect on
the change set.

### 1.4 Combining writes

The change set of a statement is the union, per constructor relation,
of the inserted sets and of the deleted sets of its writes, together
with the rows the delete actions of §7 derive.

Several writes may reach one key. After the union:

- several deletes of the same row, explicit or derived by `cascade`,
  are one delete;
- a key whose pre-state row is deleted and for which a row is inserted
  is a replacement of that row: an update when the inserted row has the
  same constructor, a retag when it has another. A delete and an insert
  of the same key in one change set therefore mean the same as the
  update or retag that produces the inserted row;
- two different rows inserted with one key fail the statement with
  `KeyViolation` ([catalog.md](catalog.md#4-error-sums)), reported at
  the statement boundary
  ([constraints.md §4](constraints.md#4-when-constraints-hold)). Two
  updates of one row to different values are such a case;
- an insert never replaces a row. An inserted row whose key is the key
  of a pre-state row that the change set does not delete fails the
  statement with `KeyViolation`, whatever its value, even when it
  equals the pre-state row.

```adt
-- users holds { id: users.key(1), name: "a" }: KeyViolation
insert users { id: users.key(1), name: "a" }
```

A new row whose key or unique value equals that of a row committed by
another transaction after this transaction's snapshot fails the
transaction with a serialization failure, not a key violation
([transactions.md §6.3](transactions.md#63-waiting-on-a-key)).

### 1.5 Faults

A fault while computing a change set (overflow, division by zero, a
failed conversion) aborts the statement as a fault in a query does
([queries.md](queries.md#15-faults)). No part of its change set is
applied.

When more than one binding, write or `fail` of one statement would
abort, the statement's outcome is any one of those aborts; the written
order of the items does not choose it
([semantics.md §6.6](semantics.md#66-statements)).

Rulings: write/surface/txdata, write/primitive/delta, write/unit/multi, write/bulk/from-query, spec/writes/insert-existing/violation, spec/writes/let-write/four-forms, spec/writes/abort-choice/any-abort

## 2. Insert

### 2.1 Forms

An insert names its target and gives its rows.

```adt
-- given n: Text, u: Key users, g: Text
insert users { name: n }
insert identities.Google { user: u, gid: g }
insert identities.Google from (
  from x in imports.Oauth select { user: x.user, gid: x.sub })
insert identities from (
  from x in imports
  select match x
    | Email e -> EmailPass { user: e.user, email: e.addr, hash: e.h }
    | Oauth o -> Google { user: o.user, gid: o.sub })
```

| Target | Single row | From a query |
| --- | --- | --- |
| A constructor relation `r.C` | `insert r.C { f: e, ... }`, a record of the fields of `C` | `insert r.C from (q)`, `q` a set of such records |
| A relation variable of a one-constructor sum `r` | `insert r { f: e, ... }` | `insert r from (q)` |
| A relation variable `r` of a sum | none | `insert r from (q)`, `q` a set of constructor values of the sum |

The record given for `r.C` holds the fields of `C`, the sum's header
fields and the key fields, minus those the relation supplies by default
(§2.3). A field that is neither given nor defaulted is a type error. A
field given that `C` does not have is a type error.

In the `from` form, `q` is checked against `Set` of the target's row
type, or of the sum or constructor set when the target is one, so an
anonymous record or constructor value in its `select` takes that type
([typing.md](typing.md)).

Inserting into the sum takes constructor values, so the constructor of
each row is data computed by the query. A record without a constructor
inserted into the relation variable of a sum with more than one
constructor is a type error.

A constructor set `r.(C | D)` or a named set `r.S` is a target like the
sum, restricted to the set: `insert r.(C | D) from (q)` takes
constructor values of `C` or `D`, and a value of another constructor is
a type error.

### 2.2 What an insert returns

An insert bound by `let` binds the rows it would insert, after defaults
and generated keys are filled in:

- the single-row form binds one row of the target's row type
  (`Identity.Google` for `identities.Google`);
- the `from` form binds the set of inserted rows, of the target's row
  type, or of the sum when the target is the relation variable of a sum,
  which a `match` consumes.

```adt
-- given n: Text, g: Text
transact
  let u = insert users { name: n }
  let i = insert identities.Google { user: key u, gid: g }
  return key i
```

`key (insert users { name: n })` is the key of the row a nested single
row insert would insert, and is itself a write: the insert is part of
the statement's change set. It is allowed only where it is evaluated
once per statement: under a comprehension, in an `update ... in`, in an
upsert's else branch, or in the scope of a match arm's binder, it is a
type error.

### 2.3 Defaults and generated keys

Defaults and generated values are clauses of the relation variable
and are applied by inserts into it ([derived.md](derived.md#4-defaults)):

```adt
relation logins : Login
  key id generate sequence
  default Revoked.at = generate clock
```

A field with a default or a generated value may be omitted from an
insert into that relation variable; the insert fills it. A field given
explicitly overrides its default. Defaults are applied only by writes
into the relation variable that declares them: an insert, and the new
value of an update or a retag, for the own fields it omits (§4.1). A value constructed anywhere else, in a query or into another
relation variable of the same type, gives every field. The value
`generate clock` takes is defined in [model.md](model.md#73-generated-keys).

A key names fields the type declares, of any type. A self-identifying
key field has type `Key r` for its own relation variable `r`; `Key r`
is a type distinct for every relation variable, represented by the key
fields' value ([model.md](model.md#7-keys)). `generate sequence` or
`generate uuidv7` after a self-identifying key makes inserts fill it
when it is not given:

| Clause | Draws | Promise |
| --- | --- | --- |
| `key id generate sequence` | a 64-bit integer | distinct only |
| `key id generate uuidv7` | a UUIDv7 as RFC 9562 defines it, with a 48-bit millisecond timestamp | distinct only |
| `default f = generate clock` | an `Instant` for a non-key field, as [model.md](model.md#73-generated-keys) defines | the wall-clock time at which the statement applies, one value for every row of the statement |

A key without `generate` must be given by every insert. A value of
`Key r` is built with `r.key(v)`, or `r.key(f: v, g: w)` for a
composite key, read back with `value(k)`, and compared with `key x` of
a row ([model.md](model.md#7-keys)); no literal checks against `Key
r`, whatever the generator.

Rules for both:

- A value is drawn when the insert is computed, so a `let` binding and
  a nested `key (insert ...)` see it within the statement.
- Values are distinct across all transactions, committed or aborted.
  A value drawn by a transaction that aborts is never issued again, so
  sequences have gaps.
- No order between generated values is promised, neither the order of
  transaction start nor of commit nor of serialization.
- A `uuidv7` key encodes the millisecond at which it was drawn; a
  `sequence` key reveals how many values were drawn before it. No
  generator hides either.
- A generated field is an ordinary field: an insert may give it
  explicitly, as an import does (`insert users { id: users.key(42),
  name: n }`). A generated `sequence` key differs from every value the
  generator has issued and from every key value that any committed row
  of the relation has held, explicit ones included.

### 2.4 Constructors not open for writes

An insert or a retag into a constructor that is declared but not yet
enabled, or that is deprecated, fails the statement with `Rejected {
reason: NotOpen {..} }`. A constructor of a value-level sum is gated
the same way: a write that stores a value of a constructor that is
declared or deprecated, in any field at any depth of any row, fails
with `Rejected { reason: NotOpen {..} }` naming that constructor,
unless the pre-state row with the same key holds an equal value at
the same field path. An update of an
existing row that keeps its constructor is allowed, also in a
deprecated constructor
([evolution.md](evolution.md#34-staged-changes)). Readers are upgraded
to the version that declares a constructor before any writer may
create its rows or store its values.

### 2.5 Rejected rows

An insert from a query may end with `rejects into j`, where `j` is a
relation variable of the prelude type `RejectedRow`
([interop.md §4.5](interop.md#45-rejects)). The source must be a
comprehension with a single generator over a source relation or an
external source, so that each row has one source record; any other
source with `rejects into` is a type error:

```adt
type GoogleRow = { id: Key identities, user: Key users, gid: Text }
relation google_rejects : RejectedRow key (load, line)

insert identities.Google from (
  from r in read_csv("google.csv", GoogleRow)
  select Google { id: r.id, user: r.user, gid: r.gid })
rejects into google_rejects
```

Each record of the source is converted on its own: read at the row
type the source names, then computed by the query. A record fails when
it does not convert to that row type or when its computation reaches
`reject e`. Every row computed from a record that does not fail is
inserted into the target. For every record that fails, one row is
inserted into `j`, holding the record's position in the source's
stated order as `line`, the source, the record as text, and the error
as `reason`. `load` is one fresh value per statement, the same in every
row the statement inserts into `j`, and distinct from every `load`
value issued before. Both sets of rows belong to the statement's one
change set.

Any other fault aborts the statement (§1.5). A constraint violation
aborts the whole statement, its rejected rows included. Without
`rejects into`, a statement with a failing record fails with
`Rejected { reason: BadInput {..} }`.

Rulings: write/insert/ctor, write/insert/value, write/insert/returns/rows, derived/defaults/on-relation, types/keygen/sequence, types/keygen/uuidv7, types/keygen/promise/distinct, types/keygen/import/advance, types/keygen/leak/live, spec/writes/nested-insert/once, spec/writes/sequence-distinct/issued-and-held, spec/writes/rejects/per-row, spec/writes/rejects-source/one-source, spec/evolution/deprecated-value/same-path

## 3. Update

### 3.1 Forms

```adt fragment
update x in R where p set e
update x set e
```

`R` is a constructor relation, a relation variable, or a query whose
rows are rows of one relation variable (a key-preserving query, such as
a comprehension over one relation variable with filters and joins whose
`select` is the binder itself). `x` is bound to each row of `R` in the
pre-state that satisfies `p`; `where p` may be omitted. `e` is the new
whole value of the row and may read `x`. The second form updates the
one row `x` already names. That `x` must be bound to a row of the
snapshot: by a generator of a query, by an upsert's `old` (§6), or by a
binder of a match over such a row. A name bound by `let` to the rows of
a write is not such a row, and `update x` on it is a type error.

```adt fragment
transact
  let u = insert users { name: n }
  update u set User { u with name: m }   -- type error
```

Bound by `let`, an update binds the set of its new rows, with fills
applied (§4.1).

```adt
-- given u: Key users, g2: Text
update g in identities.Google where g.user == u set Google { g with gid: g2 }

update i in identities where i.user == u
set match i
  | EmailPass e -> EmailPass { e with hash: rehash(e.hash) }
  | Google g    -> g
  | Apple a     -> a

update g in (from g in identities.Google, x in imports.Oauth
             where g.gid == x.sub select g)
set Google { g with user: u }
```

### 3.2 The new value

`e` must be a constructor value of the sum of `R`'s relation variable.
Over a constructor relation, `e` is typically a value of the same
constructor. Over the sum, `set match x | ...` gives each constructor's
new whole value; the match is exhaustive like any match, and an arm
that leaves a row unchanged returns its binder (`| Google g -> g`).

There is no partial update: no clause sets some fields and keeps the
rest. `C { x with f: v }` builds the whole new value from `x`. A field
holding a value-level sum is replaced by a whole new value or rebuilt
by a match over it:

```adt
-- given k: Key customers
type Address = US { zip: Text, street: Text } | Intl { country: Text, postcode: Text }
type Customer = { id: Key customers, name: Text, address: Address }
relation customers : Customer key id generate sequence

update c in customers where c.id == k
set Customer { c with address: match c.address
                                 | US a   -> US { a with zip: "02139" }
                                 | Intl i -> i }
```

A path that sets one field inside a constructor of a value-level sum
does not exist.

### 3.3 The key does not change

The new value of a row has the row's key. A new value written as a
constructor expression may not give a key field; one that does is a
type error. Every other new value whose key differs from the row's
fails the statement with `Rejected { reason: KeyChanged {..} }`
([catalog.md](catalog.md#4-error-sums)). Changing a row's identity is a
delete and a separate insert of a new row.

```adt fragment
update g in identities.Google where g.user == u
set Google { g with id: k }   -- type error
```

### 3.4 The old row is in scope

`x` and every match binder over it are bound to the pre-state row, so
the new value is computed from the old one in the same write, without a
separate read.

Rulings: write/update/ctor-binder, write/update/match-set, write/nested/replace, write/nested/rebuild, write/retag/old/in-scope, write/bulk/from-query, spec/writes/bound-row/snapshot-row, spec/writes/new-key/ctor-expr

## 4. Retag

### 4.1 Retag is an update to another constructor

There is no retag statement. An update whose new value is a constructor
other than the row's current one retags the row:

```adt
-- given k: Key identities
update i in identities where i.id == k
set match i
  | EmailPass e -> Google { gid: lookup(e.email) }
  | Google g    -> g
  | Apple a     -> a
```

The new value of an update is a value of its constructor `D`, the same
whether the row was a `D` before or not. A constructor application is
the new value of an update when it is the expression after `set`, or an
arm result of a `match` there; elsewhere every field must be given
([typing.md](typing.md)). In it:

- the key and the header fields of the sum may be omitted, and keep
  the old row's values;
- an own field of `D` that is omitted takes the relation's default for
  `D`, or its generated value, as an insert into `r.D` fills it;
- an own field of `D` with no default or generated value must be given.
  This is decided by the type checker.

`D { x with f: v, ... }` copies every field `x` has: its key and header
fields, and `D`'s own fields when `x` is a `D`. It fills only the
fields `x` lacks, as above. An own field of `D` that some constructor
of `x`'s type lacks and that has no default must be given. No field of
another constructor `C` is ever carried into `D`, whatever its name and
type.

```adt
-- given u: Key users
-- at takes the relation's default whether s was Active or Revoked
update s in logins where s.user == u set Revoked { }
```

### 4.2 What a retag guarantees

A retag of row `x` from `r.C` to `r.D`:

- keeps the row's key; references of type `Key r` and of any
  constructor set holding both `C` and `D` still reach it;
- removes the row from `r.C` and adds it to `r.D` in one step: every
  snapshot sees the row in exactly one of them, and snapshots taken
  before the commit see the old row in `r.C`;
- denotes `x` in `deleted(r.C)` and the new row in `inserted(r.D)`, so
  constraints, stored views, the change stream and audit see a deletion
  from one constructor relation and an insertion into another under one
  key;
- is checked for the new constructor's uniques and outgoing references
  as an insert into `r.D` is;
- drops the references held in fields of `C`, which no longer exist;
- runs no delete action (§7). The row was not deleted.

### 4.3 References to the old constructor

A reference whose type is a constructor set holding `C` but not `D`
cannot reach a `D` row. A retag fails the statement while such a
reference to the row exists at the end of the statement. Every
reference is checked against the constructor relation it names, so a
reference of type `Key identities` is unaffected by any retag and a
reference of type `Key identities.EmailPass` refuses one.

```adt
-- given k: Key identities, g: Text
-- refused while a reset references identity k
update i in identities.EmailPass where i.id == k set Google { gid: g }
```

A statement that also deletes or retags the referrers in the same change
set succeeds.

Concurrent transactions cannot jointly create such a reference: a retag
and a concurrent insert of a referrer typed to the old constructor do
not both commit ([transactions.md §1](transactions.md#1-serializability)).

### 4.4 Privileges

A retag is privileged by its source and target constructor sets, apart
from the privileges to insert and delete
([security.md](security.md#3-grants)).

Rulings: write/retag/assign, write/retag/old/in-scope, fk/retag/recheck, model/one-key, engine/retag-fk/by-isolation, repl/cdc/retag/ctor-pair, security/transition/transition, spec/writes/update-fill/value

## 5. Delete

```adt fragment
delete x in R where p
delete x
```

`R` is as for update (§3.1). Every row of `R` in the pre-state that
satisfies `p` is in the deleted set of its constructor relation. The
second form deletes the one row `x` names, which must be bound to a
row of the snapshot as for `update x` (§3.1). Bound by `let`, a delete
binds the set of rows it names, as they were in the pre-state, and not
the rows its delete actions derive.

```adt
-- given u: Key users, g: Text
delete s in logins.Revoked where s.user == u
delete s in (from s in logins.Active, i in identities.Google
             where s.identity == key i and i.gid == g select s)
```

A delete also derives the delete actions of §7 on every row that
references a deleted row.

Rulings: write/bulk/from-query, write/primitive/delta, spec/writes/bound-row/snapshot-row, spec/writes/let-write/four-forms

## 6. Upsert

### 6.1 Form

```adt
-- given u: Key users, g: Text, k: Key identities
insert identities.Google { user: u, gid: g }
unless conflict on (gid) in Google
else update old set Google { old with user: u }

insert identities.Google { id: k, user: u, gid: g }
unless conflict on key
else match old
  | Google o    -> update o set Google { o with gid: g }
  | EmailPass e -> fail "identity is email"
  | Apple a     -> fail "identity is apple"
```

An upsert is an insert followed by `unless conflict on K else w`. `K`
names one key or unique of the target relation variable: `key` for its
key, or the fields and constructor set of a declared unique, `(gid) in
Google`. A `K` that names no declared key or unique is a type error.
The insert's target constructor set must be a subset of the
constructor set `K` ranges over; otherwise the upsert is a type error.
For a unique over expressions, `K` is written as the unique is
declared, and two rows conflict when the unique's expressions evaluate
to equal values on them:

```adt
-- given u: Key users, e: Text, h: Bytes
-- with add unique (lower(email)) in identities.EmailPass
insert identities.EmailPass { user: u, email: e, hash: h }
unless conflict on (lower(email)) in EmailPass
else update old set old
```

### 6.2 Meaning

For each row the insert would insert, the conflict test looks for a
row with the same value of `K`:

- if there is none, the row is inserted;
- if the conflicting row is in the statement's snapshot, the row is
  not inserted, and `w` is evaluated with `old` bound to the
  conflicting row as it is in the snapshot;
- if the conflicting row was committed by a concurrent transaction and
  is not in the snapshot, the transaction fails with a serialization
  failure ([transactions.md §6.1](transactions.md#61-serialization-failures));
- if the only conflict is with a row another write of the same
  statement inserts, no `old` is bound, `w` is not evaluated, and the
  statement fails, as two inserted rows with one key or one unique
  value fail it: with `KeyViolation` when `K` is the key, and with
  `Violated` naming the unique by its canonical name when `K` is a
  unique. Among several violations of one statement the one reported
  is chosen by name ([constraints.md §7](constraints.md#7-checking-and-violation-reports)).

`old` has the row type of the constructor set `K` ranges over: the sum
for `key`, `Identity.Google` for `(gid) in Google`. A `match old` names
every constructor of that set
([typing.md](typing.md#92-exhaustiveness)). `w` is a write expression; its
change set joins the statement's. `fail e`, where `e` is an
expression of type `Text`, is a write expression that fails the
statement with a user error carrying that text
([catalog.md](catalog.md#4-error-sums)).

Two rows of one upsert's source with the same value of `K` conflict
only with each other, so the statement fails, with `KeyViolation` or
`Violated` as above.

A conflict test that meets a concurrent transaction still inserting the
same value of `K` waits for it to end
([transactions.md §6.3](transactions.md#63-waiting-on-a-key)). If it
aborts, the row is inserted; if it commits, its row is not in the
snapshot and the transaction fails with a serialization failure. The
result is serializable like any other statement.

In the else branch, `new` is bound beside `old` to the row the insert
proposed, with defaults and generated values filled in, so a bulk
upsert can read each incoming row:

```adt
insert identities.Google from (
  from x in imports.Oauth select { user: x.user, gid: x.sub })
unless conflict on (gid) in Google
else update old set Google { old with user: new.user }
```

Bound by `let`, a single-row upsert binds exactly one row. When the row
is inserted, it binds the inserted row. When it conflicts, it binds the
row with `old`'s key among the else branch's effects when the branch
replaces `old`, and `old` itself when the branch leaves it in place. The
binding is read off the change set, whatever form the branch is
written in. Its type is the join of the target's row type and the
types of the new values the branch writes, so a branch that retags
binds a row of the joined constructor set. When the else branch fails
with `fail`, the statement is rejected and nothing is bound. A `let`
on a single-row upsert whose else branch can delete the row, because
any arm of it is a delete, is a type error, since the binding must be
exactly one row. This makes a get-or-create:

```adt
-- given u: Key users, x: Text
transact
  let g = insert identities.Google { user: u, gid: x }
          unless conflict on (gid) in Google
          else update old set old
  return key g
```

The `from` form binds the set of rows it inserted together with the
binding, as above, of each conflicting row.

The else branch is required. An upsert that should leave a conflicting
row unchanged says so with `else update old set old`.

Rulings: write/upsert/unless-conflict, write/upsert/cross-ctor/match-else, engine/wait-inserter, spec/writes/upsert-scope/subset, spec/writes/upsert-binding/change-set, spec/writes/violation-name/canonical-least, spec/writes/insert-existing/violation

## 7. Delete actions

### 7.1 Declaring an action

Each reference field states what deleting its target does to the row
holding the reference:

```adt
type Badge = { id: Key badges, owner: Key users on delete restrict, label: Text }
type Reset = { id: Key resets, identity: Key identities.EmailPass on delete cascade, token: Bytes }
type Login { id: Key logins, user: Key users on delete cascade }
  = Active  { identity: Key identities on delete retag Revoked {} }
  | Revoked { at: Instant }
type Note = { id: Key notes, author: Option (Key users) on delete clear, body: Text }
```

| Action | Effect on each referrer of a deleted row |
| --- | --- |
| `restrict` | none; the statement fails if the referrer still exists at its end |
| `cascade` | the referrer is deleted in the same change set |
| `retag C { ... }` | the referrer is retagged to constructor `C` of its own sum with the given payload, in the same change set |
| `clear` | the field, of type `Option (Key ...)`, is set to `None` in the same change set |

`clear` is admitted only on a field of type `Option (Key r.S)`; on any
other field it is a schema error. A reference without a declared action
is `restrict`.

`retag C { ... }` is admitted only when `C` is a constructor of the
referrer's sum that lacks the reference field; on a header field it is
a schema error. The payload may omit the key and header fields, which carry over. It is a record of literals giving every field
of `C` that has no default or generated value; the others are filled
as for a retag (§4.1). The referrer keeps its key and header fields.
With the `logins` relation of the preamble, deleting an identity
retags each `Active` login referencing it to `Revoked`, with `at`
generated by the relation's default. The payload does not read the
referrer's old fields. Two `retag` actions on the fields of one type
must be the same action, to the same constructor with the same
payload; otherwise the type is a schema error when it is declared.

### 7.2 When an action runs

Only deletion triggers an action: an update, and a retag of the target
(§4.3), trigger none. The gone keys of a statement are the least set
of keys that holds every key of a pre-state row that the statement's
own writes leave without a row, and that is closed under `cascade`: a
pre-state row that the statement neither replaces nor deletes, whose
`cascade` field names a gone key, has a gone key itself.

Delete actions reach only rows of the pre-state that the statement's
own writes neither replace nor delete. Each such row whose key is not
gone and whose reference fields name
gone keys takes one new value from every action that reaches it, all at
once:

- if a `retag D { ... }` action reaches it, it is retagged to `D` with
  that payload, and every `clear` field that reaches it and that `D`
  still has is set to `None`;
- otherwise, every `clear` field that reaches it is set to `None`;
- a `restrict` field changes nothing.

A row deleted by `cascade` is deleted, whatever other action also
reaches it. A row the statement inserts, whose key has no row in the
pre-state, is reached by no action: if it references a gone key it
violates its reference constraint, whatever the action of the field.
A row the statement replaces is judged by its new value in the same
way: if the new value references a gone key, it violates its reference
constraint. So one statement may repoint a reference and delete its
old target.
Delete actions are the only writes the engine derives; there are no
triggers or rules.

The writes an action derives combine with the statement's own writes
as in §1.4.

`restrict` is checked once the change set is complete, at the end of
the statement, like every constraint. A statement that deletes a user
and that user's badges together succeeds; one that deletes only the
user fails, and the failure names the reference and the referrers.

```adt
-- given u: Key users
transact
  delete d in badges where d.owner == u
  delete x in users where key x == u
```

Rulings: write/delete-action/restrict, write/delete-action/cascade, write/delete-action/retag-referrer, derived/rules/none, constraint/timing/multi-assign, spec/writes/action-compose/one-value, spec/writes/action-reach/pre-state, spec/writes/action-identical/identical-only, spec/writes/action-own-writes/statement-wins

## 8. Bulk writes

Every write is set at a time: an insert takes its rows from a query, an
update and a delete range over a query. There is no loop whose body is
a single-row write. A load of external data is an insert from a query
over the file or a stream of batches, and is one change set
([interop.md](interop.md#4-loading)).

Rulings: write/bulk/from-query
