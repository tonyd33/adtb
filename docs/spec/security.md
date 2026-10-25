# Security

This part specifies who a caller is and how the engine learns it,
what a role may do and how privileges are granted, how registered
queries carry their author's rights to callers, what the engine
reveals about rows a role cannot read, what it audits and how audit
and change history are made tamper-evident, how a database file is
encrypted at rest, and how tenants are isolated. The wire exchanges
that carry authentication are in [wire.md](wire.md#3-connection-lifecycle);
bundles, registration and the ad hoc role are in
[applications.md](applications.md).

Examples extend the running schema with invoices:

```
type Invoice { id: Key invoices, owner: Key users, total: Decimal 2 }
  = Draft
  | Sent { at: Instant }
  | Void { at: Instant }
relation invoices : Invoice key id generate sequence
```

## 1. Principals

### 1.1 The Principal sum

Every call runs on behalf of a principal, a value of the sum
`Principal`. The schema declares it once, with exactly these three
constructors:

```
type Principal = EndUser { user: Key users }
               | Service { name: Text }
               | Anonymous
```

`Service` and `Anonymous` have the shapes shown. The fields of
`EndUser` are the schema's choice: they are what an application
asserts about an end user (§1.3). A schema that declares no
`Principal` has `type Principal = EndUser { name: Text } | Service {
name: Text } | Anonymous`.

Because `Principal` is a sum, every query that decides on the caller
matches it exhaustively and decides the anonymous case explicitly. A
schema change that adds a field to `EndUser` is an ordinary operator on
the type and rechecks every query that matches it.

### 1.2 `caller`

In every registered query and registered transaction, in every
schema function, and in ad hoc text, the name `caller` is a value of
type `Principal`: the principal the call runs for. It is an implicit
parameter: no client sets it, it appears in no generated signature,
and no argument can supply or override it. Its value comes from the
session as §1.3 states; a schema function sees the `caller` of the
call that evaluates it. `caller` is not in scope in constraints,
stored views, defaults or generated fields, and a schema function that
reads `caller` may not be reached by an index, a constraint, a stored
view, a default or a generated field.

```
query my_invoices () -> Set Invoice =
  from v in invoices
  where match caller
          | EndUser p -> v.owner == p.user
          | Service _ -> false
          | Anonymous -> false
  select v
```

### 1.3 Where the principal comes from

| Context | `caller` |
| --- | --- |
| A call on a connection, with no assertion | `Service { name: r }` where `r` is the role the connection authenticated as |
| A call or transaction carrying an assertion | The asserted value, an `EndUser` or `Anonymous` |
| An in-process call with no assertion | `Service { name: r }` where `r` is the schema-owner role, which the handle acts as ([embedding.md](embedding.md#22-identity-of-the-handle)) |

An assertion is a value of `Principal` sent with a single call or with
Begin for a whole interactive transaction
([wire.md](wire.md#5-calls)), or passed in the call options in
process. Only a role holding the `assert` grant (§3.2) may send one;
from any other role the call is refused with `Denied`. An assertion of
a `Service` value is refused with `Denied`: a service principal is
always the authenticated role. The engine trusts the asserting role
for the asserted value; it does not verify it. Each asserted call is
audited with both the role and the asserted principal (§8.1).

Rulings: security/principal/sum, security/assert/connection, security/assert/asserted, security/rights/deputy/caller-bound

## 2. Authentication

### 2.1 Roles

A role is a named catalog object of one database file. Every
connection authenticates as one role; every privilege is granted to a
role. The schema-owner role (§4.6) is created with the file. Roles are
created, altered and dropped by ad hoc statements
([applications.md](applications.md#7-the-ad-hoc-role)) of a role
holding `schema`; over a server the role also needs `adhoc`. Grants
are of two kinds (§3.2): relation grants are operators in migrations,
and the others are ad hoc statements.
`alter role R password p` takes the password as a parameter `p`, so
the secret never appears in statement text:

```
create role web
alter role web password p
drop role web
```

A role's credentials are SCRAM verifiers, never plaintext, kept in
`catalog::credentials`, on which no role holds a grant unless the
schema owner grants it. Roles, credential verifiers and the grants
that are deployment state (§3.2) are not part of the schema version's
hash; a dump carries them in its security section, together with the
audit relation and its ledger, and a restore reinstates them. Grants
that are operators are restored by replaying the log
([interop.md](interop.md#6-dump-and-restore)).

### 2.2 Sources

Each listener of a server states which authentication sources it
accepts; they stack per listener.

| Source | Transport | What proves the role |
| --- | --- | --- |
| Password | TCP, Unix socket | SCRAM-SHA-256 against the role's stored verifier; on TCP, with channel binding to the TLS session |
| Certificate | TCP | The client certificate presented in the TLS handshake, mapped to a role by the listener's certificate mapping |
| Peer | Unix socket | The OS user of the peer process, mapped to a role by the listener's peer mapping |

A mapping is a list of rules from a certificate subject or an OS user
name to a role name; a credential matching no rule fails
authentication. A failure never reveals whether the role exists or
which check failed. Every attempt, successful or not, is audited.

TCP connections are TLS from the first byte ([wire.md](wire.md#12-tls));
password authentication over TCP without channel binding is refused.

An embedding program opens the file directly and authenticates no one:
its handle acts as the schema-owner role and holds every privilege
([embedding.md](embedding.md#22-identity-of-the-handle)).

The certificate and peer mappings are file settings, moved and dumped
with the file; the listener addresses and the sources each listener
accepts are open-time settings of the server process
([catalog.md](catalog.md#10-configuration)).

Rulings: net/authn/scram, net/authn/mtls, net/authn/peer, net/tls/direct

## 3. Grants

### 3.1 Subjects of a privilege

Privileges are granted on two kinds of subject. Application roles are
granted registered queries and schema functions: calling one is the
privilege, and nothing else of the database is reachable to them. The
ad hoc role is granted constructor sets of relation variables, checked
against what its text reads and writes. Bundle owners hold
constructor-set grants too, because registration checks them (§4.2).

There are no row policies attached to relations. Which rows a call may
see or change is decided by the registered query it runs, written with
`caller` (§1.2).

### 3.2 Grant forms

The privileges are `read`, `insert`, `update`, `delete`, `retag`,
`call`, `adhoc`, `assert`, `schema`, `monitor`, `cancel`, `maintain`,
`configure`, `read_files` and `write_files`; there are no others.

Grants are of two kinds:

- Relation grants (`read`, `insert`, `update`, `delete`, `retag` on
  constructor sets) and `grant call fn` are schema operators. They are
  written in migrations, appended to the log by a role holding
  `schema`, covered by the schema version's hash, printed with the
  schema, and restored by replaying the log.
- Capability grants (`adhoc`, `assert`, `schema`, `monitor`, `cancel`,
  `maintain`, `configure`, `read_files`, `write_files`) and call grants
  on bundles are deployment state. They are ad hoc statements, not in
  the log and not hashed, and a dump carries them in its security
  section. Capability grants are given by a role holding `schema`; a
  call grant on a bundle is given by the bundle's owner or by the
  schema owner.

An operator grant written as an ad hoc statement is refused, and so is
a deployment grant written in a migration.

| Grant | Kind | Example | Allows |
| --- | --- | --- | --- |
| `call` on a bundle | Deployment | `grant call web-2.4 to web` | Calling every query, transaction and imported function of the bundle |
| `call` on one query | Deployment | `grant call web-2.4::login_key to support` | Calling that query |
| `call` on a family | Deployment | `grant call web::login_key to app` | Calling that query in every bundle of the family `web` |
| `call` on a schema function | Operator | `grant call fn display_name to web` | Calling the schema function directly (§4.7) |
| `read` | Operator | `grant read identities.EmailPass (user, email) to support` | Reading the listed fields of rows of the constructor set |
| `insert` | Operator | `grant insert identities.Google to web_owner` | Inserting rows of the constructor set |
| `update` | Operator | `grant update invoices.Draft (total) to web_owner` | Replacing rows of the set with rows of the same constructor that differ only in the listed fields |
| `delete` | Operator | `grant delete invoices.Void to web_owner` | Deleting rows of the set |
| `retag` | Operator | `grant retag invoices.Draft -> invoices.Sent to web_owner` | Changing a row's constructor between the sets (§5) |
| `adhoc` | Deployment | `grant adhoc to analyst` | Sending ad hoc text, queries and writes ([applications.md](applications.md#7-the-ad-hoc-role)) |
| `assert` | Deployment | `grant assert to web` | Asserting an end-user principal (§1.3) |
| `schema` | Deployment | `grant schema to deployer` | Appending migrations to the operator log, and role and capability statements (§2.1, §4.6) |
| `monitor`, `cancel`, `maintain` | Deployment | `grant monitor, cancel to oncall` | Operational commands ([catalog.md](catalog.md#9-operational-commands)) |
| `configure` | Deployment | `grant configure to dba` | Changing file settings with `set setting` ([catalog.md](catalog.md#10-configuration)) |
| `read_files`, `write_files` | Deployment | `grant read_files to loader` | Reading files within `file_roots`, and writing exports within `export_roots`, through a server ([interop.md](interop.md#3-file-scans)) |

A bundle name is `FAMILY-VERSION`, where `VERSION` is the digits and
dots after the last hyphen: `web-2.4` is version `2.4` of the family
`web`. A call grant that names a family covers every bundle of it,
registered then or later, for each granted query whose signature
(parameter and result types) in the later bundle equals the one
granted; a query whose signature changed needs a new grant.

Any grant may end in `with grant option`, which lets the grantee grant
it on. `revoke` takes the same forms and is of the same kind as the
grant it revokes. A relation grant names a relation variable and a
constructor set, written explicitly or by a named set; the relation
variable alone means every constructor of it, including constructors
added later. A grant on a named set covers a constructor that joins
the set when the constructor is added
([evolution.md](evolution.md#42-add-a-constructor)); a grant on an
explicit list covers exactly the listed constructors.

### 3.3 Field lists

A `read` or `update` grant may list fields after the constructor set.
Without a list it covers every field the constructors have when the
grant is applied; a field added later is covered only by a new grant. A listed field is either a
header field of the sum, named once for the whole set, or a field
every constructor of the set declares. To list fields that only some
constructors have, grant each constructor separately. `insert`,
`delete` and `retag` take no field list.

A query reads a field when it projects it, compares it, aggregates it,
groups or orders by it, follows a reference held in it, or returns a
row or binder whole. A query that reads a field outside the role's
grants is refused, naming the field. Matching a row and using only
granted fields of its binder reads only those fields. The key field of
a relation is granted implicitly with any `read` on the relation, and
is read whenever a row is ranged over.

### 3.4 Grants are exhaustive over the granted set

A query that ranges over, follows a reference into, tests the
existence of, or aggregates over a constructor relation the role is
not granted is refused. A role granted part of a sum ranges over the
part it holds, and its matches are exhaustive over that part:

```
grant read identities.EmailPass (user, email) to support
grant read identities.Google (gid) to support

from i in identities.(EmailPass | Google)
select match i | EmailPass e -> e.email | Google g -> g.gid      -- admitted
from i in identities.EmailPass select i.hash                     -- refused: reads EmailPass's hash field

from i in identities select i                                    -- refused: reads identities.Apple
from i in identities.Google select i                             -- refused: reads Google's user field
```

The example starts at version 12, after migrations 0011 and 0012 have
added and enabled `Apple`.

No query is admitted whose meaning would depend on rows being hidden
from it. Adding a constructor to the sum leaves `support`'s queries
unchanged unless the constructor joins a set `support` was granted by
name.

### 3.5 When grants are checked

Ad hoc text is checked against the role's grants each time it is
checked against the schema. A bundle is checked against its owner's
grants at registration and again whenever the schema or a grant
changes. A revoke that would leave a live bundle reading or writing
what its owner no longer holds is refused, naming each such bundle,
until the bundle is released: retired, superseded or expired, the
same release condition a migration waits on
([evolution.md](evolution.md#32-checks-at-apply)). The same holds for
a revoke that removes a grant option some live bundle's registration
relied on.

### 3.6 Grants across schema changes

Structural operators carry grants to the constructors they produce
([evolution.md](evolution.md#4-operators)):

| Operator | Effect on grants |
| --- | --- |
| Rename | Every grant follows the renamed constructor, field or set |
| Split | Each new constructor receives the old constructor's grants on the fields its routing carries whole, under their new names; a field the routing computes is not covered |
| Merge | The merged constructor receives a grant only where every source constructor was granted the same fields under it; otherwise the migration is refused unless grant operators in the same migration state the merged constructor's grants |
| Nest, unnest | Grants are carried to the fields as moved |
| Remove | Grants on the removed constructor or field are dropped |

Applying a migration rechecks every live bundle's owner against the
grants as they stand after it, and the migration is refused if a
bundle would read or write what its owner no longer holds.

Rulings: security/subject/queries, security/subject/relations, security/grain/ctor-set, security/fields/field-list, security/grain/partial/refuse, security/policy/in-queries, net/adhoc/principal, ops/admin/role/capabilities

## 4. Registered queries and definer rights

### 4.1 Definer rights

Every bundle has an owner, the role that registered it. A registered
query or transaction runs with its bundle owner's relation grants: a
caller needs only the `call` grant on the bundle or on the query, and
the query reaches every row its text selects within the owner's
grants. Registered queries are the only path by which an application
role touches data.

```
grant call invoices::my_invoices to web
```

`web` can run `my_invoices` and nothing else; the rows it gets are
those the query selects for `caller`.

### 4.2 What registering requires

The registering role must hold, with grant option, every relation
grant the bundle uses: `read` on every constructor set and field its
queries read, `insert`, `update` and `delete` on what its transactions
write, and `retag` on every pair of constructor sets its updates can
move a row between (§5). Registration is refused otherwise, and the
refusal lists every missing grant.

```
register bundle "bundles/web.adb"
refused: admin_dump reads identities.EmailPass (hash); web_owner lacks read with grant option
```

`register bundle` is a command ([catalog.md](catalog.md#91-commands)),
issued here by `web_owner`; `adt register` wraps it.

### 4.3 Parameters do not choose the principal

A definer query returns whatever its parameters select to anyone
holding its call grant. The principal a query authorizes against is
`caller`, which no parameter can supply (§1.2). A query that takes a
key as a parameter and should return only the caller's rows must test
`caller` itself:

```
query invoice (k: Key invoices) -> Set Invoice =
  from v in invoices
  where v.id == k
    and match caller
          | EndUser p -> v.owner == p.user
          | Service _ -> false
          | Anonymous -> false
  select v
```

### 4.4 Retiring a bundle

A live bundle is retired by the command `retire bundle web-2.3`,
issued by its owner or by the schema owner. No other role may retire
it, whatever other privileges it holds. Retirement is deployment state
in `catalog::bundles`, not an operator in the log.

### 4.5 Ad hoc rights

Ad hoc text runs with the rights of the role that sends it. Its reads
are checked against that role's own grants (§3.4), and its writes,
including a `transact` block sent as one ad hoc call, need the role's
`insert`, `update`, `delete` and `retag` grants for what they write.
Constraints hold for ad hoc writes as for any other.

### 4.6 The schema owner

One role per database file is the schema owner. Only it, and roles it
grants `schema` to, may append migrations to the operator log
([evolution.md](evolution.md#1-the-operator-log)). The schema owner owns
every relation variable and holds every relation grant on it with
grant option; every other relation grant derives from it.

### 4.7 Schema functions called directly

A bundle that calls a schema function directly imports it with
`import fn display_name`, which gives the function an ordinal in the
bundle. A connection bound to that bundle may then call it when its
role holds the call grant on the whole bundle, or `grant call fn
NAME`. The function runs with the
schema owner's relation grants, and with `caller` bound to the
caller's principal (§1.2), so it authorizes rows as a registered query
does.

```
grant call fn display_name to web
```

Rulings: security/rights/definer, security/rights/deputy/caller-bound, security/register-priv/holds-reads, security/retire-priv/owner-or-admin, security/schema-priv/owner

## 5. Retag privileges

Changing a row's constructor is privileged separately from creating
and deleting rows. `grant retag r.S -> r.T` allows a write to replace
a row of a constructor in `S` with a row of a different constructor in
`T` of the same relation variable. `insert` on `T` and `delete` on `S`
neither imply nor are implied by it. An update that keeps the
constructor needs `update` on that constructor.

```
grant retag invoices.Draft -> invoices.Sent to web_owner
grant retag invoices.(Draft | Sent) -> invoices.Void to billing_owner
```

The grants form a state machine over the constructors. A registered
transaction is checked at registration: for every `update` over the
sum, each arm of its `set match` names the constructor it reads and
the constructor its result has, and every pair where they differ must
be covered by a `retag` grant of the bundle owner. In

```
txn send (k: Key invoices, t: Instant) -> Key invoices =
  transact
    update v in invoices where v.id == k
    set match v
      | Draft d  -> Sent { d with at: t }
      | Sent s   -> s
      | Void x   -> x
    return k
```

the only pair is `Draft -> Sent`. Arms returning their binder leave
the row unchanged and need no grant. A referential action that retags a referrer
([writes.md](writes.md#7-delete-actions)) runs with the rights of the
schema owner, which declared it.

Rulings: security/transition/transition

## 6. Leaks

### 6.1 Constraint violations

A change set that violates constraints fails with `Violated`, which
names one constraint: the violated constraint with the smallest name.
Its report holds the rows of that constraint's denial query that the
caller may read, and the number of rows redacted: rows that touch a
constructor set or field the caller may not read are left out, and
carry no key, constructor or field. For a registered call the caller's
rights are its bundle owner's; for ad hoc text they are the sending
role's own grants ([wire.md](wire.md#11-errors)). In process nothing is
redacted, since the handle holds every privilege.

### 6.2 Statistics, plans and conflicts

Statistics, query plans and conflict details describe rows. Reading
any `sys::` relation, `catalog::plans` or the plan of another
principal's query needs `monitor`, which shows counts, timings and
plan shapes. A column whose values reveal keys or field values of a
constructor relation also needs `read` on that constructor relation's
rows and fields: `value` of `sys::field_values`, `keys` of
`sys::conflicts`, `witness` of a `Violation` in `sys::verify_findings`,
and the probe bounds and literal parameter values in `text` and
`document` of `catalog::plans` and of explain results. A query that
reads such a column without that `read` is refused with `Rejected {
reason: Denied {..} }`; a query that reads only the other columns is
not ([catalog.md](catalog.md#14-visibility)). Conflict details beyond
the caller's own side of an aborted transaction need `monitor`
([catalog.md](catalog.md#64-conflicts)).

### 6.3 What is out of scope

The engine does not hide timing, the occurrence of serialization
aborts, or resource use. What a registered query reveals through its
result, its faults or a uniqueness refusal is the behaviour of that
query, and is reviewed with its bundle.

Rulings: security/leak/opaque-errors, security/leak/hidden-stats, ops/conflicts/leak/admin-only

## 7. Catalog visibility

The catalog is relations ([catalog.md](catalog.md#1-system-relations)),
under the same grants as any others: there is no filtering of catalog
rows by role, and rows are never silently omitted. A query reading a
column the role may not see is refused. An application reads the catalog only through registered
queries, whose bundle owner must hold `read` on the catalog
constructor sets they read. The ad hoc role is granted catalog
constructor sets like any others:

```
grant read catalog::constructors to analyst
```

Rulings: security/catalog/open

## 8. Audit

### 8.1 What is audited

The engine records:

| Event | Recorded with |
| --- | --- |
| Authentication, successful or not | Role tried, source, listener, outcome |
| Grants and revokes, role changes | Statement, by whom |
| Bundle registration and retirement | Bundle, version, owner, by whom |
| Every schema operator appended to the log | Version produced, author |
| Every call of a registered query, transaction or schema function, and every ad hoc call | Connection role, `caller`, bundle and query or the ad hoc text, arguments, outcome |
| Every call refused for lack of a privilege | Connection role, `caller`, bundle, query |
| Every committed change set | Commit position, `caller`, bundle, query, the change set |

### 8.2 The audit relation

Audit records are rows of the catalog relation `catalog::audit`,
durable and serializable like any catalog relation, whose type is a
sum of event kinds:

```
type AuditEvent { seq: Key catalog::audit, at: Instant, role: Text, principal: Text }
  = Login    { source: Text, outcome: Outcome }
  | Grant    { statement: Text }
  | Register { bundle: Text, version: I64 }
  | Retire   { bundle: Text }
  | SchemaOp { version: I64 }
  | Call     { origin: Origin, args: Bytes, outcome: Outcome }
  | Denied   { origin: Origin }
  | Commit   { position: I64, origin: Origin, changes: Bytes }
  | Truncate { through: I64, digest: Bytes }
type Origin = Registered { bundle: Text, query: I32 }
            | Function   { name: Text }
            | AdHoc      { text: Bytes }
type Outcome = Succeeded | Failed { error: Error }
```

Every row carries the time it was written, the connection role and
the principal in a frozen form: the canonical text of the `Principal`
value ([interop.md](interop.md#1-textual-value-syntax)), holding no
reference, so deleting a user or changing `Principal` leaves audit
rows as written. A failed login records `Anonymous`. `origin` says what
ran: `query` is the ordinal of the registered query or transaction in
its bundle, `name` a schema function called directly, and `text` the
BLAKE3-256 hash of ad hoc text. `args`
is the BLAKE3-256 hash of the call's arguments in the wire encoding at
the bundle's pinned version ([wire.md](wire.md#4-value-encoding)).
`changes` is the change set in that encoding, or at the current
version for an ad hoc commit. Because `changes` holds whole rows, a
query may read it only under the column rule for catalog relations:
with `read` on every constructor set and field the change set covers
([catalog.md](catalog.md#14-visibility)). `outcome` is
`Succeeded`, or `Failed` with the error value returned
([wire.md](wire.md#11-errors)). `position` is the commit position, and
`version` the schema version produced or pinned. `Truncate` records an
archive and truncation through the sequence number `through`, with
the digest that covers it. A bundle's queries are named by ordinal; the name is in the
catalog row of the bundle at the recorded version. The engine writes audit records in its own system
transaction, which commits whether or not the audited transaction
does: a denied or aborted call is recorded. No role may insert,
update or delete rows of `catalog::audit`; it is read through grants like
any relation, and matched exhaustively like any sum.

The engine never prunes `catalog::audit`. A role holding `maintain`
and `write_files` may archive and then truncate a prefix of it with
the command `archive audit`, up to a sequence number covered by a
ledger digest that has been exported (§8.3); the archive and the
truncation are recorded as an audit event themselves. The command is
available over the server and in process, and `adt audit archive`
wraps it.

```
archive audit through 991204 to "audit-2026-10.arrow"
```

### 8.3 Ledger

Commit change sets, schema operators and audit records are hashed into
a chain of blocks:

- Each committed transaction contributes an entry: the BLAKE3-256
  hash of the wire encoding ([wire.md](wire.md#4-value-encoding)) of
  the audit rows its system transaction wrote, its `Commit` row, which
  holds the commit position and the change set, included. BLAKE3-256
  is the function schema version hashes use.
- A block closes after 1024 commits or 1 second, whichever comes
  first, and on clean shutdown. Its hash is the BLAKE3-256 of the previous
  block's hash, the block number, the first and last commit position,
  and the Merkle root of its entries in commit order.
- The head digest is the block number, the last commit position and
  the block hash.

The digest is exported by an operational command and by a periodic
push to a configured sink, to be kept outside the database. Verifying
recomputes the chain from the file and its log archive and compares it
with a digest kept elsewhere:

```
$ adt ledger digest app.adt
block 9912  position 3/1204411  3f9a...
$ adt ledger verify app.adt --digest digests/2026-10.txt
```

Tamper evidence detects a change to history before the last exported
digest; it does not prevent one.

Rulings: security/audit/auth, security/audit/ops, security/audit/calls, security/audit/writes, security/audit-store/relation, security/tamper/ledger

## 9. Encryption at rest

### 9.1 What is encrypted

A database file is created encrypted or not; the choice is a format
setting ([catalog.md](catalog.md#10-configuration)). In an encrypted
database the pager encrypts every page as it writes it and decrypts it
as it reads it: heap, index, catalog, large-value, free-space and
allocation pages alike. Pages are plaintext in the buffer pool and
ciphertext on disk. The redo log is encrypted too, full-page images
included. Only the file header, kept as two alternating copies on
pages 0 and 1 and holding format settings and the wrapped data keys,
is plaintext ([engine.md](engine.md#22-file-header)).

Query memory beyond its budget spills first to pages of the buffer
pool, which are encrypted as any page when written, and then to spill
files in the spill directory, encrypted under a key generated per
process and never stored.

### 9.2 Cipher and nonces

Pages and log blocks are sealed with XChaCha20-Poly1305. A log block's
number, its batch sequence number and its batch's block count stay in
clear as its associated data, so recovery can find whole batches
without the key. Each write of
a page draws a fresh random 192-bit nonce from a cryptographic random
source and stores it with the page, so a page rewritten without a new
log position still never reuses a nonce under its key. The page
number, the page's log position, its format and its kind stay in
clear in the page header, as associated data. The associated data
binds the file id, the page number, the page's log position and the
data key version, so a page moved to another place or another file,
or with its log position altered, fails its tag.

A page replaced by an older valid ciphertext of itself carries an
older log position under a valid tag. It is detected only by the
check of page log positions against the log during recovery
([engine.md](engine.md#26-corruption-detection)); outside recovery,
detecting such a rollback is out of scope.

The 16-byte authentication tag replaces the page checksum: a page
whose tag fails is a corrupt or forged page and is handled as a
detected corruption ([catalog.md](catalog.md#111-verification)). A database
created without encryption keeps a 32-bit checksum per page instead.

Writes encrypt into a staging buffer; the frame in the pool stays
plaintext while the write is in flight, so concurrent optimistic
readers never see ciphertext. A read decrypts into the frame before the
frame is published to readers.

### 9.3 Key hierarchy

A key-encryption key (KEK) wraps data keys (DEKs). Each DEK is 256
random bits with a version number; the header holds every DEK version
still in use, wrapped by the KEK. Every page and log block names the
DEK version it was sealed with. New writes use the newest DEK.

The KEK comes from one of three sources, chosen when the file is
created and recorded in the header:

| Source | Holder | Header records |
| --- | --- | --- |
| Passphrase | A person or the server's secret store | KDF parameters and salt |
| KMS | An external key management service that wraps and unwraps DEKs | The key's URI and version |
| Host bytes | The embedding program, from a platform keystore | Nothing but the kind |

Opening an encrypted file requires its KEK; the header holds a check
value sealed under each DEK, so a wrong key is reported as
`WrongKey`, distinct from corruption. A passphrase is turned into
the KEK by Argon2id, with the salt and parameters the header records.

### 9.4 Rotation

Rotating the KEK rewraps every DEK under the new KEK and rewrites only
the header. Rotating the DEK adds a new version used by every later
write; pages under older versions are rewritten as they are next
written, and a background pass rewrites the rest, found by the version
each page names. A DEK version may be destroyed only when no page, no
retained log segment and no retained backup uses it; the command is
refused otherwise, naming what still uses it. Key commands are
commands of [catalog.md](catalog.md#91-commands), available over the
server and in process, and need the `configure` grant; `adt key`
wraps them.

```
rotate key kek "kms://vault/adt-app/v3"
rotate key dek
destroy key dek 7
refused: backup 2026-10-01 uses dek 7
```

### 9.5 Backups and dumps

Backups copy pages and log as stored, still encrypted
([engine.md](engine.md#99-backups-and-point-in-time-restore)). A backup's manifest lists the DEK
versions it uses and their wrapped forms. Restoring requires the KEK
those were wrapped under and every listed DEK version.

A logical dump ([interop.md](interop.md#6-dump-and-restore)) is
plaintext, whether or not the database is encrypted, unless it is
written with `--encrypt KEY` (`dump to "path" encrypt key`), which
seals it under that key. A dump needs `maintain` and `read` on every
relation it dumps; its security section (roles, verifiers, deployment
grants) is written only for a holder of `schema`.

Rulings: security/encrypt/page, security/encrypt/io-cost/staging, security/integrity/aead, security/nonce/random, security/keys/wrapped, security/kek/passphrase, security/kek/kms, security/kek/host, security/backup-crypt/as-stored

## 10. Tenancy

### 10.1 A file per tenant

Tenants are isolated by giving each its own database file. Each file
has its own catalog, roles, grants, operator log, bundles, keys,
backups, conflicts and resource accounting, so no tenant can read,
collide with, or slow another through the database. A server serves
many files and a client names its tenant's file at connect
([wire.md](wire.md#1-transports-and-the-server)). Deleting a tenant is
deleting its file and destroying its keys. There is no tenant field,
no tenant key prefix and no row policy.

### 10.2 Fleet operations

A fleet is a directory of tenant files that share one schema history.
In the example the tenants start at version 10. The fleet tool applies one migration to each file and registers one
bundle in each:

```
$ adt migrate --fleet tenants/ 0011
acme   ok     at 11 (c7d2...)
beta   ok     at 11 (c7d2...)
gamma  refused: log differs from 10 (a91c...) at 7
$ adt register --fleet tenants/ bundles/web.adb
```

Each file is changed in its own transaction; a refusal in one leaves
the others applied. Registration is deployment state in each file's
`catalog::bundles`, not an entry in its log, so registering never makes
tenant logs differ. A file already at the target version is skipped,
and a file whose log differs from the migration's predecessor is
refused, so the tool may be rerun until every file reports `ok`. Since
a version is identified by its position and the hash of its log
prefix, one bundle pins every tenant whose log is identical up to that
version.

Rulings: security/tenancy/file-each, security/tenancy/fleet/fleet

## 11. Information flow

The engine controls access only. It decides which constructor sets
and fields a query may read and which writes it may make; it does not
label data or track where values go after a query reads them. What a
registered query may pass from what it reads to its result or its
writes is fixed by its text, reviewed with its bundle.

Rulings: security/ifc/none
