# Applications

This part defines how application code reaches the database: named,
typed registered queries and transactions; bundles, the unit in which
a client release registers them and pins a schema version; leases,
supersession and retirement, by which the engine knows which readers
are still live; what binding a connection to a bundle means; what a
schema change does to registered bundles; the ad hoc role; how an
embedded database is opened, migrated and registered; the printed
schema; and modules.

The operator log, versions, translation and readers-first are in
[evolution.md](evolution.md). The frame format of the handshake is in
[wire.md](wire.md), the in-process API in [embedding.md](embedding.md),
and privileges in [security.md](security.md).

Examples start from version `12 (c41e...)` of the history in
[evolution.md](evolution.md#example-histories), at which `Apple` has
been added (`0011`) and enabled (`0012`), with sessions:

```adt
module auth
type User = { id: Key users, name: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
type Session { id: Key sessions } = Active { who: Key identities on delete retag Revoked {} }
                                  | Revoked { at: Instant }
relation users : User key id generate sequence
relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
  unique (sub) in Apple
relation sessions : Session
  key id generate uuidv7
  default Revoked.at = generate clock
set OAuth of identities = Google | Apple
```

## 1. Registered queries and transactions

### 1.1 What is registered

Application code runs registered queries and registered transactions.
A registered query is a named, typed catalog object: a `query`
declaration with its name, its declared parameters and its result
type. A registered transaction is a `txn` declaration likewise. A
client calls either by its ordinal in the bundle with arguments; it
never sends query text, except as an ad hoc call (§7).

```adt
query login_key (u: Key users) -> Set (ByEmail { email: Text } | ByGoogle { gid: Text } | ByApple { sub: Text }) =
  from i in identities where i.user == u
  select match i
    | EmailPass e -> ByEmail { email: e.email }
    | Google g    -> ByGoogle { gid: g.gid }
    | Apple a     -> ByApple { sub: a.sub }

txn sign_up (n: Text, g: Text) -> Key identities =
  transact
    let u = insert users { name: n }
    let i = insert identities.Google { user: key u, gid: g }
    return key i
```

A registered query is a function of its parameters and the snapshot it
runs at. A registered transaction is a function from its parameters
and its snapshot to a change set and a result
([transactions.md](transactions.md#3-transact-blocks)).

### 1.2 Checked and planned at registration

Registration parses, type checks and checks exhaustiveness of every
query and transaction at the schema version its bundle pins (§3),
and plans each one. The plan is stored in the catalog with the query.
A call never parses, type checks or plans; it runs the stored plan.
When the engine replaces a plan (on a schema change or a statistics
change, [catalog.md](catalog.md#7-plans)), the replacement is made
before any call uses it, and the query's meaning does not change.

A query of a new bundle whose checked form equals the checked form of
the same-named query of an earlier bundle of the same family (§3.1)
inherits that query's plan, and its plan pin if it has one.

A failed check refuses the registration and names the query and the
failure. A registered query never fails at call time with a type or
exhaustiveness error.

Rulings: lang/registered/named, exec/plan-cache/at-register, surface/registration/build-time

## 2. Where definitions live

There are two homes for named definitions.

### 2.1 Schema functions

A schema function is a `fn` defined by `define fn` in a migration
([evolution.md §4.11](evolution.md#411-definitions)). It is a schema
object: it changes only through the log, and every schema change
rechecks it. A view is a schema function with no parameters
([derived.md §1](derived.md#1-views)).

A schema function is also a schema-resident query. A bundle that lets
its clients call one imports it with `import fn NAME`, which gives it
an ordinal in the bundle like a declared query; it is called at the
bundle's pinned version, with the definition of that version. A
principal calling it, by the bundle's ordinal or directly, needs
`grant call fn NAME to R`; a call grant on a bundle covers its
registered queries and transactions only. A schema function called
this way runs with the schema owner's rights, bound to the caller
([security.md](security.md#4-registered-queries-and-definer-rights)).

```adt
-- 0013, from version 12
module auth
define fn logins (u: Key users) -> Set Text =
  from i in identities where i.user == u
  select match i
    | EmailPass e -> e.email
    | Google g    -> g.gid
    | Apple a     -> a.sub
```

```adt
bundle web-2.6 pins 13 (5b9a...) edition "2026-10"
import auth
import fn logins
```

### 2.2 Bundle functions

A `fn` declared in a bundle file belongs to that bundle. Its
queries and transactions may call it; clients may not. It may be
generic, with type parameters as a schema function has. It is checked
with the bundle at the bundle's pinned version, and rechecked with the
bundle on a schema change (§6). A bundle function whose name equals a
schema function's name visible in the bundle at its pinned version is
refused at registration. A bundle's names resolve at its pinned
version, so a migration may define a schema function with the name of
a bundle function, and the bundle's uses of that name still denote its
own function.

```adt
bundle web-2.3 pins 12 (c41e...) edition "2026-10"
import auth
fn emails_of (u: Key users) -> Set Text =
  from i in identities.EmailPass where i.user == u select i.email
query contact (u: Key users) -> Set ({ email: Text }) =
  from e in emails_of(u) select { email: e }
```

Rulings: lang/defs-home/schema, lang/defs-home/bundle, surface/registration/schema-resident

## 3. Bundles

### 3.1 The bundle file

A bundle is a named client release: its registered queries and
transactions, its bundle functions and imported schema functions, and
the one schema version they are checked against. It is the unit of
registration, of version pinning, of lease, of supersession and of
retirement. A bundle file starts with its header:

```adt
-- bundles/web.adb
bundle web-2.4 pins 12 (c41e...) edition "2026-10" lease 90d
import auth
import fn display_name
query login_key (u: Key users) -> Set Text =
  from i in identities where i.user == u select display_name(i)
query sessions_of (u: Key users) -> Set Session.Active =
  from s in sessions.Active where s.who.user == u select s
```

| Header part | Meaning |
| --- | --- |
| `bundle NAME` | the bundle's name, `FAMILY-VERSION`, unique in the database |
| `pins V (H)` | the schema version, position and hash, every declaration is checked at |
| `catalog V (H)` | the catalog version, required to read catalog relations ([evolution.md §10](evolution.md#10-engine-releases)) |
| `edition "E"` | the spec edition the bundle is written in, required |
| `lease D` | the lease duration, a `Duration` (§4.2) |
| `hide new constructors` | the bundle never blocks `enable r.C`, though it still blocks `enable T.C` for a field it reads; rows of constructors its version lacks are invisible to it, and following a reference to one faults `NoSuchRow`; rows of known constructors are never hidden ([evolution.md §8.1](evolution.md#81-enabling-a-constructor)) |

The priority and resource clauses are in
[catalog.md](catalog.md#83-limits-and-deadlines); the full header
grammar is in [syntax.md](syntax.md). The bundle has no owner clause:
its owner is the role that registers it. The bundle's row of
`catalog::bundles` records its owner, edition, schema and catalog
pins, lease and state.

A bundle name is `FAMILY-VERSION`, where VERSION is the digits and
dots after the last hyphen: `web-2.4` is version `2.4` of family
`web`. Releases of one application form one family.

`query`, `txn`, `import fn` and `feed` declarations get ordinals in the
order they appear in the file, from 1, so a code generator computes
them from the file alone; bundle functions and module imports take
none. The qualified name of a registered query is the bundle name and
the query name: `web-2.4::login_key`.

A family's owner is the role that registered its first bundle. Only
that owner or the schema owner registers a bundle into the family. A
call grant names a bundle or a family (`grant call web::login_key to
app` covers the bundles of `web` whose `login_key` has the signature
the grant records, [security.md §3.2](security.md#32-grant-forms));
call grants are deployment state, given by the bundle's or family's owner
or the schema owner ([security.md](security.md#3-grants)).

### 3.2 Bundles are immutable

A bundle name is registered once. Registering a bundle whose name is
registered with identical content is a no-op for a live bundle; for a
superseded or expired one it runs checks 3 to 5 of §4.1 and, when they
pass, makes the bundle live again with a new lease (§4.2, §4.4). With
different content it is refused. A
retired bundle's name is never registered again. A new client release
is a new bundle with a new name.

### 3.3 A bundle states what it reads

Because a bundle pins a version and the engine holds all its queries,
the engine knows, for every live bundle, which constructor sets it
reads and writes and at which version. Readers-first and translation
are decided from this ([evolution.md §7](evolution.md#7-version-translation),
[§8](evolution.md#8-readers-first)). Feeds are owned by bundles and
encoded at their bundle's version ([wire.md](wire.md)); registered
change consumers are registered, pinned and leased as bundles are, and
count as readers likewise.

Rulings: surface/registration/unit/bundle, surface/reader-tracking/bundles

## 4. Bundle lifecycle

Registration, leases, supersession and retirement are deployment
state, rows of `catalog::bundles`. None of them is an operator in the
schema log or changes a schema version.

### 4.1 Registration

A bundle is registered at deploy, before the client release that uses
it runs:

```sh
$ adtb register bundles/web.adb
login_key    ok
sessions_of  ok
registered web-2.4 at 12 (c41e...)
```

`adtb register` wraps the command `register bundle`
([catalog.md](catalog.md#91-commands)).

Registration is refused, as a whole, when:

1. the pinned version does not exist in the log, or its hash differs,
   or likewise for the pinned catalog version (the report gives the
   position and both hashes);
2. any declaration fails to check at the pinned version (§1.2);
3. the pinned version is older than the current one and some query or
   transaction has no translation to the current version
   ([evolution.md §7.2](evolution.md#72-translation-is-decided-at-apply));
4. the pinned version lacks a constructor that is enabled now, and the
   bundle reads the whole relation variable holding it, follows a
   reference whose current type admits it, or reads a field whose type
   reaches the value-level sum holding it
   ([evolution.md §8.1](evolution.md#81-enabling-a-constructor)): the refusal is
   `Rejected { reason: BundleStale {..} }`, naming the constructor; a bundle
   declaring `hide new constructors` is exempt from the relation and
   reference cases, not from the field case;
5. the registering principal lacks the privileges the bundle needs
   ([security.md](security.md#42-what-registering-requires));
6. the name is taken by different content, or by a retired bundle
   (§3.2);
7. the family has an owner (§3.1) and the registering principal is
   neither that owner nor the schema owner.

A bundle may pin a version at which a constructor is declared but not
yet enabled. Its queries must cover that constructor, and registering
it is what lets the constructor be enabled:

```adt
-- 0011 of evolution.md §1.1: Apple declared, write-disabled
```

```adt
bundle web-2.5 pins 11 (c7d2...) edition "2026-10" lease 90d
import auth
query logins_of (u: Key users) -> Set Text =
  from i in identities where i.user == u
  select match i
    | EmailPass e -> e.email
    | Google g    -> g.gid
    | Apple a     -> a.sub
```

```adt fragment
-- 0012, once no live bundle pinned at 10 or earlier reads identities whole
module auth
enable identities.Apple
```

A bundle is live from its registration until it is released: retired,
superseded or expired. Registration starts its lease. `register bundle
FILE` is the command of [catalog.md §9.1](catalog.md#91-commands);
`adtb register` wraps it.

### 4.2 Leases

A bundle holds its pinned version by lease. The lease is renewed when a
client binds a connection to the bundle (§5) and when it makes a call
on one. Renewals made at a replica are forwarded to the primary
([replication.md §3](replication.md#3-schema-leases-and-upgrades-at-replicas)).

A lease expires when the lease duration has passed since the last
renewal recorded in the log. The primary and every replica compute
expiry from that record alike. From expiry a replica refuses binds and
calls on the bundle with `Rejected { reason: BundleExpired {..} }`. The
primary treats the lease as expired only once the further grace of the
file setting `LeaseGrace` (1 hour by default) has passed. A renewal of
an expired lease, made at the primary or forwarded to it, is refused
with `BundleExpired`; no renewal extends an expired lease.

A bundle whose header has no `lease` clause takes its lease duration
from a database setting ([catalog.md](catalog.md#10-configuration)),
30 days by default.

An expired bundle is not retired, and it is not live. It stops
holding back schema changes: readers-first and translation-at-apply
ignore it. Binds, calls and renewals of it are refused with
`Rejected { reason: BundleExpired {..} }` on the primary and on every
replica. The way back is registering it again (§3.2), which runs
checks 3 to 5 of §4.1 ([evolution.md §7.3](evolution.md#73-expired-bundles))
and, when they pass, makes it live with a new lease.

### 4.3 Retirement

```sh
$ adtb retire bundle ios-4.2
```

`retire bundle B` is a command, also available as a statement to a
role that may retire `B`; it is never in a migration. It records the
retirement in `B`'s row of `catalog::bundles`. From its commit:

- binding a connection to `B` is refused with
  `Rejected { reason: Retired { bundle } }`;
- every call on a connection already bound to `B` is refused likewise;
- `B` holds back no schema change.

Retirement is never the outcome of a migration: a migration that
breaks a live bundle is refused (§6), and the bundle is retired only by
an explicit `retire bundle`. Retirement is permanent; a retired
bundle's name is never registered again.

Only the bundle's owner (the role that registered it) or the schema
owner may retire it
([security.md](security.md#44-retiring-a-bundle)).

### 4.4 Supersession

A bundle is superseded when a newer release of the same embedded
application replaces it (§8), or by the command:

```adt
supersede bundle ios-4.2 by ios-4.3
```

`supersede bundle B by B2` is a deployment command, given by `B`'s
owner or the schema owner; `B2` must be a live bundle of the same
family as `B`. A superseded bundle is not retired and not a live
reader: it holds back no schema change, and binding to it is refused
with `Rejected { reason: BundleSuperseded {..} }`. Registering it again with
identical content runs the checks of §3.2 and makes it live again.

Rulings: surface/reader-tracking/dormant/lease, surface/reader-tracking/dormant/retire, surface/lease-policy/translate-stale, security/retire-priv/owner-or-admin, surface/registration/order/pinned

## 5. Binding a connection

A connection is bound to one bundle, at the bundle's pinned version, by
its handshake ([wire.md](wire.md#3-connection-lifecycle)). The handshake
names the bundle and the version with its hash, and the catalog
version when the bundle pins one. The engine refuses the binding when
the bundle is not registered, is retired or superseded, has expired
(§4.2), or pins a different version or
hash than the handshake states; a catalog version that differs from
the bundle's catalog pin is refused with
`Rejected { reason: CatalogMismatch {..} }`. A role that may call
nothing in the bundle is refused with `Denied`, audited, and renews no
lease; `adhoc` alone does not bind.

A bound connection:

- calls the queries and transactions of its bundle and the schema
  functions it imports (§2.1), each by ordinal;
- runs every call at the bundle's pinned version, translated to the
  current version where they differ
  ([evolution.md §3.5](evolution.md#35-pinned-versions));
- takes the arguments of every call as values of its parameter types
  at the pinned version; where the pinned version is not the current
  one, the arguments map forward to the current version as the values
  of a translated write do
  ([evolution.md §7.1](evolution.md#71-serving-a-pinned-version));
- receives every result of its bundle's calls encoded at the pinned
  version: constructor
  wire tags are the catalog ids of that version, and constructors of an
  anonymous result sum are numbered by their position in the query
  signature;
- renews the bundle's lease at binding and on calls (§4.2);
- may also send ad hoc text when its principal holds `adhoc`; that text
  is checked at the schema and catalog versions of its transaction's
  snapshot, not the pinned ones, and its results are encoded at that
  schema version, their tags given by the result's descriptor (§7).
  Every statement of a transaction runs at one schema version and one
  catalog version: a call that would run at another, such as ad hoc
  text after a registered call or a registered call after ad hoc text,
  is refused with `Rejected { reason: MixedVersions }`.

A connection is bound once and serves one bundle for its lifetime. A
client of two bundles opens two connections. An embedded database
binds its handle to the application's bundle at open (§8).

Rulings: net/bind/connection, spec/txn/one-version/one-pair

## 6. What a schema change does to bundles

A migration is rechecked against every live bundle. Each query and
transaction of the bundle is translated from its pinned version to the
new version; if any has no translation, or its translation fails to
check, the migration is refused, and the report names each bundle,
query and operator:

```sh
$ adtb migrate 0014
refused: bundle ios-4.2 pins 12 (c41e...): txn link_apple inserts identities.Apple
  (0014 deprecates identities.Apple)
  retire bundle ios-4.2, or supersede it by a release without link_apple
  (supersede bundle ios-4.2 by ios-4.3);
  ios-4.2 expires only once every client stops calling it (see sys::sessions)
```

`adtb migrate` wraps the command `apply migrations`
([catalog.md](catalog.md#91-commands)).

A refused migration does not wait: it applies only when presented
again after the bundle is released (retired, superseded or expired),
or with changed operators. For the file setting `BlockingNoticePeriod` (24 hours by
default) after a refusal, the engine sends `BlockingMigration` notices
to the sessions bound to each bundle the report names
([evolution.md §8.1](evolution.md#81-enabling-a-constructor)). A
migration with staged jobs reruns every check of
[evolution.md §3.2](evolution.md#32-checks-at-apply) at its final
commit; a failure there discards the pending migration. A bundle declaring `hide new constructors` never blocks an `enable r.C`;
after one, its paths through references are partial and fault
`NoSuchRow` when they reach a hidden row. A migration never applies with
a live bundle left failing, and never retires one.

Rulings: surface/registration/on-change/refuse, evolution/breaks/invisible/registered

## 7. The ad hoc role

Query text sent by a client is accepted only from a principal holding
the `adhoc` privilege ([security.md](security.md#45-ad-hoc-rights)).
Application principals without it run only registered queries.

```sh
$ adtb repl --role analyst
adt> from i in identities select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub
```

Ad hoc text may be a query, a write or a `transact` block. Several
items in one text form one transaction, each item a statement, with
results returned in order. The first item that faults, fails, violates
a constraint or is rejected ends the text: nothing of the text
commits, later items do not run, and the error names the item's
ordinal. Free names in the text are parameters,
typed by the descriptors the caller supplies; a free name with no
supplied parameter is an error. A write
needs, besides `adhoc`, the write privileges each of its statements
needs; constraints hold as for any write. Loads, staging repairs and
one-off data fixes run this way. A `transact` block sent as one call is
an ad hoc call (`CallText` on the wire, [wire.md](wire.md#7-ad-hoc-text-and-appends)) or an
in-process call ([embedding.md](embedding.md)).

Ad hoc text:

- is checked when it runs, against the current schema version,
  including exhaustiveness;
- is outside the registered guarantee: it is not rechecked by schema
  changes, holds back none, and is not a reader for readers-first. A
  schema change between two runs of the same text is noticed only when
  the text fails to check;
- returns results in the same encoding as a registered query, at the
  current version.

Rulings: surface/adhoc/scoped

## 8. Embedded databases

An embedded application ships its migrations and its bundles compiled
into it, and opens the file with them. The open sequence is normative
in [embedding.md](embedding.md#2-opening-a-database):

```text
let db = adtb::open("app.adt", &schema::LOG, &[&queries::BUNDLE], OpenOptions::default())?;
```

After recovery, after refusing a file whose log and the shipped
migrations differ at some position both have (`Diverged`), and after
refusing a downgrade, opening:

1. records the given bundles in `catalog::bundles`, unchecked;
2. marks every bundle recorded by an earlier open and not given now as
   superseded (§4.4);
3. binds the host functions the program supplies; one whose code
   identity differs from the catalog's fails open with
   `HostFnMismatch`, unless the shipped migrations contain the
   `redefine fn` for it and no staged job or index build of a migration
   before that `redefine fn` reaches the function;
4. applies the shipped migrations the file lacks, in order, each as in
   [evolution.md §3](evolution.md#3-applying-a-migration), running
   staged jobs to completion;
5. checks and registers each given bundle at its pinned version
   (§4.1), and binds the handle.

Steps 1 and 2 commit durably together, before step 4. A later failure
does not undo them; a superseded bundle is live again only once
registered again (§3.2). A bundle recorded in step 1 is not a reader
until step 5 registers it. The registrations of step 5 are atomic: if
any fails, none is made. A downgrade, an open with an older bundle that
would read a constructor enabled since its pin or whose host function
was redefined since, fails with
`Rejected { reason: DowngradeRefused { cause } }` and changes nothing.

Open never appends to the log anything the shipped migrations do not
contain, so the next release's open compares equal logs. A superseded
bundle holds back nothing, so the shipped migrations' `enable`
operators are not blocked by the application's own earlier releases.
Other bundles registered on the file stay live and hold back changes
as on a server.

Rulings: surface/open-applies

## 9. The printed schema

The engine prints the folded schema at a version:

```sh
$ adtb schema print > schema.adt
$ adtb schema print --check schema.adt
```

`adtb schema print` wraps the command `print schema`
([catalog.md §9.1](catalog.md#91-commands)).

The printout states every module, type, relation variable with its
clauses, named set, schema function, aggregate, constraint, stored
view, default, computed field, grant and constructor write state at
that version, in the declaration syntax of [syntax.md](syntax.md),
headed by the version and its hash. Equal versions print identical
text. `--check` compares a committed printout with the fold of the log
and fails on any difference. The review sees each migration beside the
schema it produces. The printout is never input: the log is the only
authored schema.

The same rules print a single migration in canonical form: whitespace
normalised, every name qualified by its module, and carried bytes
replaced by their BLAKE3 hash. The printed form is for review; the
version hash covers the encoding of the resolved operators, not this
text ([evolution.md §2.1](evolution.md#21-position-and-hash)).

Rulings: surface/generated, spec/evolution/version-hash/value-encoding

## 10. Modules

Every schema name belongs to a module. Every module file and
migration file starts with `module NAME`, one module per file; a bundle
file starts with its bundle header instead (§3.1). Schema objects
declared in a module file, by operators or declarations, belong to its
module. Each module has one namespace for its schema-resident
definitions. Ad hoc text belongs to no module: an unqualified name in
it resolves in every user module, then the prelude
([queries.md §16](queries.md#16-ad-hoc-statements)). In a module or
bundle file, a name from another module is written qualified: `auth::identities`, `auth::Identity.Google`.
`import NAME` in a bundle file makes the names of module
`NAME` usable unqualified; an unqualified name found in two imported
modules is an error at that use. The engine's system and catalog
relations are in the modules `sys` and `catalog` (`sys::sessions`,
`catalog::constructors`). A bundle is a namespace for its declarations:
`web-2.4::login_key`.

```adt
module billing
type Account { id: Key accounts } = Owned { who: Key auth::identities }
                                  | Shared { team: Text }
relation accounts : Account key id generate sequence
```

Rulings: surface/namespace/modules
