---
title: Canonical notation for code examples in question and option bodies
---
Every code example in a question or option body uses this notation,
except where a defeated or impossible option's notation is the
alternative it describes. Each rule names the ruling it follows;
"(convention, not ruled)" marks a form no ruling decides, picked as the
one most used in accepted bodies.

## Lexical

- Strings are double-quoted: `"google"`. Single quotes are unused.
  (`syntax/strings/double`)
- `==` is equality, `!=` inequality; `=` only binds (`let x = ...`,
  `name = ...` in declarations). (`syntax/equality/double`)
- `and`, `or`, `not`. The prelude declares `type Bool = False | True`;
  `true` and `false` are literal spellings of its constructors.
  (convention, not ruled)
- `--` starts a line comment. (convention, not ruled)

## Types and relation variables

```
type Identity { id: Key identities } = EmailPass { email: Text, hash: Bytes }
                                    | Google    { gid: Text }
                                    | Apple     { sub: Text }
type User = { id: Key users, name: Text }
type Status = Draft | Sent { at: Instant } | Paid { at: Instant }
```

- `type` declares a type; constructors are listed as alternatives in
  one declaration. A record is a one-constructor type; an enum is a sum
  of nullary constructors. (`syntax/decl/type-relation`,
  `surface/sum-decl/alternatives`, `model/record-is-ctor`,
  `model/nullary-sum`)
- Fields common to every constructor are a header on the type:
  `type Identity { created_at: Instant } = EmailPass { ... } | Google { ... }`.
  (`surface/shared-fields/header`)
- Type parameters follow the name, lowercase: `type These a b = ...`.
  (convention, not ruled)
- A declared constructor order: `type Status ordered = Draft | Sent {
  at: Instant } | Paid { at: Instant }`. (`lang/sum-order/declared`; keyword:
  convention, not ruled)
- Defaults and generated values are relation clauses, applied by
  inserts into that relation variable: `default n = 0`, `default
  Revoked.at = generate clock`, `key id generate sequence`. There is no
  clock function in queries: the current time is a parameter or a
  generated value. (`derived/defaults/on-relation`, `types/now/param-only`)
- Time types: `Instant` (a point on the time line), civil `Date`,
  `Time` (time of day), `DateTime`, `ZonedDateTime`; lengths are
  `Duration` and `Period`. Integers `I8`..`I64`, floats `F32`, `F64`.
  (`model/time`, `types/durations/split`, `types/int-widths/*`)
- System and catalog relations are in modules `sys::` and `catalog::`:
  `sys::sessions`, `catalog::constructors`. (convention, not ruled)
- Grants: `grant call web-2.4::login_key to support`, `grant read
  identities.(EmailPass | Google) (user) to support`, `grant read
  identities.EmailPass (email) to support`; listed fields must be
  header fields or declared by every constructor in the set.
  (convention, not ruled)

```
relation users : User
  key id generate sequence
relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
  index (user) in (Google | Apple)
set OAuth of identities = Google | Apple
```

- `relation r : T` declares a relation variable; keys, uniques and
  indexes are clauses on it. `unique`/`index` name the fields, then
  `in` and a constructor or a parenthesised constructor set; without
  `in` they range over every constructor. A short form `relation r : T
  key id` is one line. (`syntax/keys/on-relation`,
  `surface/extent/separate`, `fk/subset/subset-index`)
- The key names fields the type declares; a key field has type `Key
  r` for its own relation (`types/id-type/key-type`), so such a type
  belongs to that one relation variable. A composite key is `key (doc,
  tag)`. `generate sequence` or `generate uuidv7` after the key makes
  inserts fill it when not given. (`types/keygen/*`; spelling:
  convention, not ruled)
- A named set after `in` needs no parentheses: `unique (sub) in OAuth`.
  A field of a value-level sum is addressed by path, `unique
  (status.Paid.ref)`, or by `tag(status)`. A range index is `interval
  index (resource, during) in (Room | Desk)`. Nothing is deferrable:
  every constraint holds at every statement boundary.
  (`constraint/timing/multi-assign`; spelling: convention, not ruled)
- `unique` is never written inside a constructor's fields.
  (`syntax/keys/on-relation`)
- A named constructor set: `set OAuth of identities = Google | Apple`,
  used as `identities.OAuth`. No `except`. (`syntax/ctor-set/paren`,
  `surface/ctor-set/named`, `surface/ctor-set/list`)

## Constructor relations and sets

- `identities.Google` is a constructor relation; `identities.(Google |
  Apple)` a constructor set; `identities` the whole sum.
  (`syntax/ctor-set/paren`, `query/ctor-relations`)
- At the type level, `Identity.Google` is the row type of a
  constructor and `Identity.(Google | Apple)` of a set, as binders and
  or-binders have. (`lang/binder/ctor-row`,
  `lang/patterns/or-binder/ctor-set`)

## References

```
type Reset = { identity: Key identities.EmailPass on delete cascade }
type Device = { owner: Key users on delete restrict }
type Session = Active { who: Key identities on delete retag Revoked {} }
             | Revoked { at: Instant }
```

- `Key r` references any row of `r`, `Key r.C` and `Key r.(C | D)` a
  constructor set; `Key r.S` widens to `Key r.T` when `S` is a subset
  of `T`. (`syntax/reference/key`, `model/typed-key`,
  `model/ref-subtyping/ctor-set-subsumption`)
- The referential action is written on the field: `on delete
  restrict`, `on delete cascade`, `on delete retag C { ... }`. No
  `set null`. (`syntax/reference/key`, `write/delete-action/*`)
- `key u` is the key of row `u`, typed `Key r` for the relation `u`
  ranges over; references compare with `==`: `i.user == key u`.
  (convention, not ruled)
- `i.user.name` follows a reference; `u.<identities.user` is the set of
  rows of `identities` whose `user` references `u`. An optional
  reference is an `Option` and is matched. (`lang/nav/paths`,
  `lang/nav/backlinks`, `lang/nav/path-option/option-typed`)

## Queries

```
from u in users, i in identities
where i.user == key u and u.name == n
select { name: u.name, login: match i
                                | EmailPass e -> e.email
                                | Google g    -> g.gid
                                | Apple a     -> a.sub }
```

- Comprehensions: `from x in r, y in s where p select e`. Results are
  sets. (`lang/family/comprehension`, `lang/collection/sets`)
- Records: `{ email: e.email }`, field names always given; record
  types `{ email: Text }`. (`syntax/record/colon`; naming every field:
  convention, not ruled)
- `match e | C x -> ... | D _ -> ...` names every constructor;
  `_` matches only a scalar, and a match on a scalar ends with a
  binder or `_` arm. Guards (`| Google g if g.domain == "corp" -> ...`)
  do not count toward coverage, so every constructor also has an
  unguarded arm. Or-patterns: `| Google g | Apple g -> g.sub`. Record
  patterns `EmailPass { email: e, verified: true }`; a field name
  alone in a pattern binds that field. (`query/compile-time`,
  `lang/wildcard/scalars-only`,
  `lang/patterns/guard-cover/ignore-guarded`, `lang/patterns/*`;
  punning in patterns: convention, not ruled)
- A nested collection is a field: `{ user: u, devices: from d in
  devices where d.owner == key u select d }`. No outer join.
  (`lang/results/nested`, `lang/outer/nested`)
- Aggregates bind: `sum(l.amount for l in lines where l.entry == key
  e)`, `count(i for i in identities where i.user == key u)`,
  `max(x.created for x in g)`. Aggregating a projected set is a type
  error. An aggregate over a possibly empty range returns `Option`.
  (`syntax/aggregate/comprehension`,
  `lang/collection/set-agg/over-binders`, `lang/empty-agg/nonempty`)
- `top 3 by p.created_at desc (from p in posts where ... select p)` is
  a set, usable at any depth; `order by` only orders a final result.
  (`lang/nested-order/top-n`, `lang/order/result-only`)
- `from i in identities group i by i.user into g select { user: g.key,
  n: count(x for x in g) }`; `g` is nonempty. `tag(i)` is the
  constructor of `i` as a nullary sum, usable as a group key.
  (`lang/group/into`, `lang/group/ctor-key/tag-type`)
- `exists x in r where p` and `not exists x in r where p` test
  emptiness; `count(x for x in r where p) == 0` is equivalent.
  (convention, not ruled)
- A parameterless `fn` is called with `()`: `active()`. (convention,
  not ruled)
- Recursion: `fix reach = q union (from r in reach, ... select ...) in
  e`. (`lang/recursion/fix`)

## Declarations

```
query emails (u: Key users) -> Set ({ email: Text }) =
  from i in identities.EmailPass where i.user == u select { email: i.email }
fn display_name (i: Identity) -> Text = match i | ...
fn active () -> Set User = from u in users where u.status == Active select u
txn sign_up (n: Text, g: Text) -> Key identities = transact ...
query search (name: Option Text) -> Set User = ...
```

- `query` (registered client query), `fn` (schema function; a view is
  a parameterless `fn`), `txn` (registered transaction); parameters by
  declared name, no sigil; `->` and the result type are required.
  Results are `Set T`, never `Bag`; a result sum anonymous to the
  query is written in the signature: `-> Set (ByEmail { email: Text } |
  ByGoogle { gid: Text })`. (`syntax/query-decl/arrow`,
  `syntax/params/names`, `lang/params/declared`, `lang/nullary-fn`,
  `lang/option-params`, `surface/query-scoped`)
- A space separates the name from the parameter list. (convention, not
  ruled)

## Writes and transactions

```
transact
  let u = insert users { name: n }
  let i = insert identities.Google { user: key u, gid: g }
  return key i

insert identities.Google { user: u, gid: g }
insert identities from (from x in imports select match x.kind
                          | Email e -> EmailPass { email: e.addr, hash: e.h }
                          | Oauth o -> Google { gid: o.sub })
update g in identities.Google where g.user == u set Google { g with gid: g2 }
update i in identities where i.id == k
set match i
  | EmailPass e -> Google { gid: lookup(e.email) }
  | Google g    -> g
  | Apple a     -> a
delete s in sessions where s.expires < t
insert identities.Google { user: u, gid: g }
unless conflict on (gid) in Google
else update old set Google { old with user: u }
insert identities.Google { id: k, user: u, gid: g }
unless conflict on key
else match old
  | Google o    -> update o set Google { o with user: u }
  | EmailPass e -> fail "key taken"
  | Apple a     -> fail "key taken"
```

- `transact` computes one change set from one snapshot; `let x =
  insert ...` binds the rows it would insert; `return` gives the
  result. (`write/transact/block`, `write/insert/returns/rows`,
  `write/unit/multi`)
- `insert` into a constructor relation, or into the sum with values
  computed by a match. (`write/insert/ctor`, `write/insert/value`)
- `update x in r where p set e`: `e` is the new whole value of the row.
  Over the sum, `set match x | ...` gives each arm's new whole value;
  an arm yielding another constructor retags the row; an unchanged row
  is its binder (`| Google g -> g`). There is no `retag` clause and no
  partial-record arm. (`write/retag/assign`, `write/update/*`,
  `write/nested/rebuild`)
- `C { x with f: v }` is `x` with field `f` replaced, as constructor
  `C`. (convention, not ruled)
- `delete x in r where p`. (convention, not ruled)
- A `transact` block with no `return` applies its change set and
  returns nothing; options follow the keyword: `transact retry 5`.
  `key (insert users { name: n })` is the key of a nested insert.
  (convention, not ruled)
- Upsert: `insert ... unless conflict on (f) in C else e`, or `unless
  conflict on key`; `old` is the conflicting row, typed by the
  constructor set the conflict ranges over (the whole sum for `key`).
  (`write/upsert/unless-conflict`, `write/upsert/cross-ctor/match-else`;
  clause spelling: convention, not ruled)

## Constraints

```
constraint balanced =
  from e in entries
  where sum(l.amount for l in lines where l.entry == key e) != 0
  select e
check email_length on identities.EmailPass (length(email) <= 254)
```

- `constraint name = query`: a named query that must return nothing;
  its rows are the violation report. (`constraint/form/empty`,
  `surface/constraint/denial`, `syntax/constraint/named`)
- `check name on r.C (expr)`: a row check over the constructor's own
  fields, named. (`surface/constraint/check`, `syntax/constraint/named`)
- Dropped by name: `drop constraint balanced`. (`syntax/constraint/named`)

## Migrations

A migration is a numbered file of operators; it applies as one schema
change and its number is the schema version it produces. `0011 add
...` is shorthand for a one-line migration file `0011`.
(`surface/source/op-log`, `surface/migration/handwritten`,
`surface/version/counter`, `syntax/operator/verb`)

```
-- 0011
add constructor Identity.Apple { sub: Text }
  joins identities.OAuth
  arm emails: Apple a -> { email: a.sub }
  arm sub_present: Apple a -> length(a.sub) > 0
add unique (sub) in identities.Apple
redefine fn display_name (i: Identity) -> Text = match i | ... | Apple a -> a.sub

-- 0012
enable identities.Apple

-- 0013
rename constructor Identity.Facebook to Meta

-- 0014
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }

-- 0015
merge constructor Identity.(Google | Apple) into OAuth
  { provider: Google | Apple, sub: Text }

-- 0016
define fn display_name (i: Identity) -> Text = ...
add constraint balanced = ...
drop constraint balanced

-- 0017
revert 0011
deprecate identities.Apple
retire bundle ios-4.2
```

- Verb, object, clauses. Operators act on a type; `joins` states which
  named sets of each relation variable a new constructor joins; `arm`
  gives the new constructor's arm of each registered query, schema
  function and match-scoped constraint that matches the sum.
  (`syntax/operator/verb`, `evolution/types`,
  `surface/ctor-set/growth/declare`,
  `constraint/scope/new-ctor/arm-in-add`)
- `split ... by match` routes every row of the old constructor by an
  exhaustive match over its row. (`evolution/split-routing/match-route`)
- `define fn`, `redefine fn`, `drop fn`, `add constraint`, `drop
  constraint`, `enable`, `deprecate`, `retire bundle` and `revert` are
  operators in the log. (`evolution/in-log`, `evolution/log-ops`,
  `evolution/revert`)
- A uniqueness or index change is `add unique (f) in r.C`, `drop unique
  (f) in r.C`. Other operators keep the verb form: `remove constructor
  Status.Suspended`, `move field Identity.EmailPass.created to
  Identity`. (convention, not ruled)
- The `match` after `by` has no scrutinee: it matches the row of the
  constructor being split. (convention, not ruled)

## Versions and bundles

```
-- bundles/web.adb
bundle web-2.4 pins 9 (a91c...) edition "2026-10"
import auth
query login_key (u: Key users) -> Set Text = ...
```

- A version is the log position, shown with its content hash: `9
  (a91c...)`. Never a bare hash, never `onto`. (`surface/version/counter`,
  `surface/both`, `surface/version/history/linear`)
- `bundle NAME pins VERSION (HASH) edition "DATE"` heads a bundle
  file, `edition` required, optional `catalog V (H)`, `lease`,
  `priority`, `budget` clauses after it; no `owner` clause (the owner is
  the registering role). The bundle is
  the unit of registration and pinning, registered at deploy and held
  by a lease. (`surface/registration/unit/bundle`,
  `surface/reader-tracking/bundles`,
  `surface/reader-tracking/dormant/lease`, `syntax/module/header`)
- A lease is a header clause: `bundle ios-4.2 pins 8 (a91c...)
  edition "2026-10" lease 90d`. A record type used as a type argument
  is parenthesised: `Set ({ email: Text })`. A query kept in the schema is a `define fn` in a migration.
  (convention, not ruled)
- Command lines are shown as `$ adt ...`. (convention, not ruled)

## Modules

```
module auth
type Identity = ...
relation identities : Identity key id generate uuidv7
```

`module NAME` heads a file, one module per file; names from another
module are qualified `auth::identities` (convention, not ruled).
(`syntax/module/header`, `surface/namespace/modules`)

## Wire

A stored constructor's wire tag is its never-reused catalog constructor
id; a constructor of a result sum anonymous to the query is numbered by
its position in the query signature, from 1. (`surface/sum-encoding/stable-tag`,
`surface/query-scoped`)
