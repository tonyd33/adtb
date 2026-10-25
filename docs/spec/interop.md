# Interoperation

This part defines how values and relations leave and enter the
database: the textual value syntax, the external formats and their
mapping of sums, file scans, loading, export and change streams, dump
and restore, the operator-facing guarantees of backup and point-in-time
restore, and the tool that migrates a SQL database in. It also defines
what code other than the query language may add: functions written in
WebAssembly or supplied by the embedding program, user-defined
aggregates, extension packages, and the index kinds built from them.

Examples use the running schema of [README.md](README.md#conventions)
at version 12, after migration 0011 of
[evolution.md](evolution.md#42-add-a-constructor) adds the constructor
`Apple { sub: Text }` to `Identity` and joins it to `identities.OAuth`,
and migration 0012 enables `identities.Apple`, extended locally. Each
section's examples are an independent history starting from that
state unless the section states another version.

## 1. Textual value syntax

### 1.1 One syntax

The textual syntax of a value is its literal syntax in the query
language ([syntax.md](syntax.md#25-literals)), and for a value with no
literal, its form in §1.3. The same syntax is used
in queries, dumps, CSV cells, test fixtures, error details, statistics
(`sys::field_values`) and the REPL. Every value of every type has a
canonical printed form, and an implementation must satisfy, for every
type `T` and value `v` of `T`:

- reading the printed form of `v` at type `T` gives a value equal to
  `v`;
- equal values print identically.

Printing follows these rules; spellings of individual literals are
those of [syntax.md](syntax.md#25-literals):

- A constructor value prints its constructor name, then, if it has
  fields, `{ f: v, ... }` with every field, header fields first, each
  in declaration order. A record of an anonymous record type prints
  only the braces. A nullary constructor prints its name.
- A key prints as the call that builds it ([model.md](model.md)):
  `users.key(3)`, or `r.key(f: v, g: w)` for a composite key. In the
  flat formats of §2 a key is written as its key value.
- A `Decimal` prints exactly as many fractional digits as its type's
  scale. A float prints the shortest decimal that reads back to the
  same value; NaN and the infinities print in the forms of §1.3. `-0.0` never occurs as a value
  ([model.md](model.md#34-floats)).
- `Text` prints in NFC with the escapes of syntax.md.
- Time values, `Period` and `Bytes` print in the forms of §1.3.
- A `Set T` value prints as `[v1, v2, ...]`, each element once, in
  ascending order by `T`'s order ([model.md](model.md#10-order)), or by
  the canonical key bytes of the elements when `T` is not ordered. The
  empty set prints as `[]`. A reader accepts the elements in any order
  and merges equal ones.

Text is always read at a known type: a document states the relation
whose rows it holds, and a cell or field is read at the declared type
of its column. Literals take their type from that context
([typing.md](typing.md#8-literals-and-numeric-types)), so a printed value carries no type
annotations beyond what the syntax requires.

Every type `T` has a function `T.parse (t: Text) -> Option T` that
reads one value in the textual syntax, with constructor names read at
the caller's version (the pinned version for a registered call), and
every value has `print (v: T) -> Text`
([prelude.md](prelude.md)). `(Tag Identity).parse("Google")` is
`Some { value: Google }`.

### 1.2 Versioned documents

Constructor names are not identity: a constructor keeps its id when
renamed. A text document stored outside the database therefore states
the edition of the textual syntax and the schema version its names are
written at, in a first line:

```text
text 2026-10 pins 42 (c7f2...)
data users
User { id: users.key(3), name: "ann" }
User { id: users.key(4), name: "bo" }
```

A reader reads a document at its stated version and translates its
rows to the reader's version through the operator log, by the same
translation that serves a pinned bundle
([evolution.md](evolution.md#71-serving-a-pinned-version)). Renames,
splits, merges, nests and field operators are translated as they are
for a bundle; where a step of translation is lossy, the read fails
with `Rejected { reason: Untranslatable {..} }` naming the operator.
Names are mapped through the log, never matched as text, so a name a
split or remove freed and a later constructor reused is not confused
with the old one. A document without a first line is read at the
reader's version.

Single values shown to a caller (error details, REPL output) are
printed at the caller's version and carry no header.

Rulings: interop/text/literal, interop/text/rename/versioned

### 1.3 Values with no literal

These values have no literal in the query language. Their textual
forms are:

| Value | Form | Example |
| --- | --- | --- |
| `F32`, `F64` NaN and infinities | `NaN`, `inf`, `-inf` | `inf` |
| `Instant` | ISO 8601 date and time in UTC, `Z` suffix | `2026-10-25T09:30:00.000000Z` |
| `Date` | ISO 8601 calendar date | `2026-10-25` |
| `Time` | ISO 8601 time of day | `09:30:00.000000` |
| `DateTime` | ISO 8601 date and time, no offset | `2026-10-25T09:30:00.000000` |
| `ZonedDateTime` | its `DateTime`, then the zone name in brackets | `2026-10-25T09:30:00.000000[Europe/Paris]` |
| `Period` | `P{m}M{d}D`, its month and day counts as signed integers | `P1M0D`, `P0M-15D` |
| `Bytes` | `0x`, then two hex digits per byte | `0x`, `0x00ff` |

- A time prints with exactly six fractional digits of a second. A
  reader also accepts fewer, or none with no `.`.
- A year outside `0000` to `9999` prints with a sign and at least five
  digits, the expanded form of ISO 8601.
- `Bytes` print lowercase hex; a reader accepts either case. The empty
  value is `0x`.

In query source each form is the argument of a `T.parse` call (§1.1),
which returns `Option T` ([syntax.md](syntax.md#25-literals)):

```adt fragment
Instant.parse("2026-10-25T09:30:00.000000Z")   -- Some { value: ... } : Option Instant
Period.parse("P1M0D")
Bytes.parse("0x00ff")
```

Rulings: spec/textual-forms/interop, spec/bytes-form/hex

## 2. External formats

### 2.1 Formats

The engine reads and writes four formats. Each is a reader for file
scans and loads (§3, §4) and a writer for export (§5.1).

| Format | Files | Sums |
| --- | --- | --- |
| CSV | RFC 4180, with a header row; columns are matched by name | tag columns, or text cells (§2.3) |
| NDJSON | one JSON value per line | the JSON mapping (§2.4) |
| Parquet | Parquet files | tag columns (§2.3) |
| Arrow IPC | the Arrow IPC stream and file formats | dense unions |

### 2.2 Type mapping

| Type | Arrow | Parquet | CSV cell |
| --- | --- | --- | --- |
| `Bool` | `bool` | `BOOLEAN` | `true`, `false` |
| `I8` .. `I64` | `int8` .. `int64` | `INT32`/`INT64` with the matching integer annotation | textual syntax |
| `F32`, `F64` | `float32`, `float64` | `FLOAT`, `DOUBLE` | textual syntax |
| `Decimal s` | `decimal128` with scale `s` | `DECIMAL` with scale `s` | textual syntax |
| `Text` | `utf8` | `BYTE_ARRAY` annotated `STRING` | the text |
| `Bytes` | `binary` | `BYTE_ARRAY` | textual syntax |
| `Instant` | `timestamp[us, UTC]` | `TIMESTAMP(MICROS, UTC)` | textual syntax |
| `Date` | `date32` | `DATE` | textual syntax |
| `Time` | `time64[us]` | `TIME(MICROS)` | textual syntax |
| `DateTime` | `timestamp[us]` with no zone | `TIMESTAMP(MICROS)` not UTC-adjusted | textual syntax |
| `Duration` | `duration[us]` | `INT64` microseconds | textual syntax |
| `Period`, `ZonedDateTime` | `utf8` in the textual syntax | `STRING` in the textual syntax | textual syntax |
| `Key r` | the mapping of the key's value; a reader builds the key with `r.key(v)` | likewise | likewise |
| newtype over `B` | the mapping of `B` | likewise | likewise |
| record | `struct` | group | one column per field, named `f.g` by path |
| `Option T` | `T`, nullable | `T`, optional | an empty unquoted cell is `None`; a quoted empty cell is `Some { value: "" }` |
| enum (every constructor nullary) | `dictionary<int32, utf8>` of constructor names | `STRING` | the constructor name |
| other value-level sum | dense union (§2.3) | tag columns (§2.3) | tag columns or a text cell (§2.3) |
| nested set in a result | `list<struct>` | `LIST` | refused |

Arrow IPC files keep the dictionary of constructor names for an enum
other than `Bool`; the Arrow format of the server protocol sends an
unsigned tag column instead ([wire.md](wire.md#54-arrow-format)).

A writer writes exactly these types. A reader converts a column to the
declared type of its field when the column's type is the one listed,
or when the column is text holding the field's textual syntax. A null
in a column whose field is not an `Option` is a conversion failure.

### 2.3 Value-level sums in flat formats

A field `f` of a value-level sum type with payloads has two flat
encodings.

Tag columns: a column `f.tag` holding the constructor name, and for
each constructor `C` and each of its fields `g` a column `f.C.g`,
empty (null) unless the tag is `C`. In Parquet these are one group `f`
with a member `tag` and one optional group per constructor. A reader
must check that exactly the tagged constructor's members are present
and every other constructor's are null; any other combination is a
conversion failure.

```adt
type Via = Email { addr: Text } | Phone { num: Text }
type Contact = { name: Text, via: Via }
```

```text
name,via.tag,via.Email.addr,via.Phone.num
ann,Email,a@b.example,
```

Text cell: one column `f` holding the value in the textual syntax.

```text
name,via
ann,"Email { addr: ""a@b.example"" }"
```

A writer uses tag columns unless asked for text cells (§5.1). A reader
reads a field from a column named `f` as a text cell when one exists,
and from tag columns otherwise.

In Arrow IPC a value-level sum is a dense union with one child per
constructor; each child field is named by its constructor's name, and
the union's type ids are the order of the children. Options and enums
use the mappings of §2.2.

A result whose row type is a sum of several constructors, such as an
export of a relation-level sum, is written with one column per header
field, a column `tag` holding the constructor name, and a column `C.g`
for each constructor `C` and each of its own fields `g`, empty unless
the tag is `C`. In Parquet the constructor fields are one optional
group per constructor; in Arrow IPC they are one dense union column
`value` with one child per constructor. A file in this shape reads
back at the sum's type (§3.1).

### 2.4 The JSON mapping

| Value | JSON |
| --- | --- |
| record | an object with one member per field |
| nullary constructor | a string, the constructor name |
| constructor with payload | an object with one member, named by the constructor, holding the payload object |
| `Bool` | `true`, `false` |
| `I8` .. `I32`, `F32`, `F64` | a number; NaN and the infinities as strings in the textual syntax |
| every other scalar | a string holding its textual syntax |
| nested set | an array |

```text
{"id": "3", "name": "ann"}
{"Google": {"id": "0192f0c4-...", "user": "3", "gid": "g-123"}}
"None"
{"Some": {"value": "a@b.example"}}
```

A JSON reader must accept exactly what the writer produces, with
object members in any order.

### 2.5 No JSON type

There is no JSON value type. JSON is only a format: a payload received
as JSON is held as `Text` or `Bytes`, or read into typed data by
`read_ndjson` (§3) at a declared type, whose sums give it its shape.

Rulings: interop/formats/csv, interop/formats/ndjson, interop/formats/parquet, interop/formats/arrow-ipc, interop/formats/flat-sums/tag-columns, interop/formats/flat-sums/text-cell, interop/json-map/external, interop/json/none

## 3. File scans

### 3.1 Reading files as relations

Four relation sources read immutable files:

```text
read_csv (path: Text, T) -> Set T
read_ndjson (path: Text, T) -> Set T
read_parquet (path: Text, T) -> Set T
read_arrow (path: Text, T) -> Set T
```

`T` is the row type: a declared record type, or a sum type read in
the shape §2.3 gives a sum's rows. The result is the set of rows the
files hold, read at `T`: each field is read from the column of the
same name (the member of the same name for NDJSON) by the rules of §2.
Columns the type does not name are ignored. A column the type names
that the file lacks is a conversion failure, except for a field of an
`Option` type, which reads `None`. Constructor names, in tag columns,
text cells and fields of a `Tag` type, are read at the file's version
(§5.1) and translated to the caller's version (the pinned version for
a registered call). A path may contain
glob patterns and names every file it matches, read as one relation;
how paths resolve, and which URL schemes are supported, is
implementation-defined.

```adt
type LogRow = { at: Instant, user: Key users, bytes: I64 }

from r in read_parquet("logs/2026-10-*.parquet", LogRow), u in users
where r.user == key u
group r by u.name into g
select { name: g.key, bytes: sum(x.bytes for x in g) }
```

The type of a file source is its declared row type, never a type
inferred from a file, so a query naming one is checked before any file
exists. A file whose content does not convert fails at read: an ad hoc
query or a registered call fails with `Rejected { reason: BadInput
{..} }` naming the file, row and column, unless the read is the source
of a load with a rejects relation (§4.5).

Through a server, a file read needs the `read_files` privilege
([catalog.md](catalog.md#93-capabilities)), held by the caller for ad
hoc text and by the bundle's owner for a registered call, and is confined to the
directories listed in the open setting `file_roots`; a path outside
them is refused with `Rejected { reason: Denied {..} }`. `file_roots`
is empty by default, so a server reads no files until it is set. A
caller of the in-process API reads any path the process can.

### 3.2 Where files may be read

A file source may appear in an ad hoc query and in the source query of
an insert (a load), ad hoc or in a registered transaction. Ad hoc text
may write: a principal holding `adhoc` and the write privileges a
statement needs runs ad hoc loads, staging repairs and ad hoc
`transact` blocks, under every constraint as always
([applications.md](applications.md#7-the-ad-hoc-role)). Each ad hoc
write is audited in `catalog::audit` with its principal
([security.md](security.md#8-audit)). It is never
a relation variable: no key, unique, index or reference may range over
a file, and a file source must not occur in a constraint, a row check,
a stored view, a default, a computed field, an index expression or a
schema function. Such a program is ill-typed and refused.

### 3.3 File integrity

The first read of a path in a transaction records the BLAKE3-256
checksum of each file it names (or the version an object store
reports), and the set of files a glob matched. Every later read of the same path in the
transaction must see the same files with the same checksums. If it
does not, the transaction aborts with `Retry { reason: FileChanged {..}
}`.

Rulings: interop/foreign/file-scan, interop/foreign/typing/declared, interop/foreign/integrity/no-constraints, interop/foreign/integrity/pinned-file

## 4. Loading

### 4.1 A load is an insert

A load is an ordinary write: an insert whose source query reads a file
(§3). It computes one change set, checked as one at its statement
boundary like any change set ([writes.md](writes.md#1-change-sets)).
There is no separate load command.

```adt
type GoogleRow = { id: Key identities, user: Key users, gid: Text }

insert identities.Google from (
  from r in read_csv("google.csv", GoogleRow)
  select Google { id: r.id, user: r.user, gid: r.gid })
```

A load may give values for generated key fields. A value the
generator later draws still differs from every value it has issued and
from every key any committed row of the relation has held, the loaded
rows included ([model.md](model.md#73-generated-keys)).

The in-process API also takes a stream of Arrow record batches into
one constructor relation, each batch in the Arrow mapping of that
constructor's row type (§2.2), and applies the whole stream as one
change set when the stream is committed
([embedding.md](embedding.md#9-bulk-append)). A batch whose schema
does not match fails at push with `SchemaMismatch`.

A remote client sends rows through an `Append` stream of the server
protocol ([wire.md](wire.md#73-append)), into one constructor
relation, staging relation or temp relation (§4.3) per stream, which
needs `insert` on it; a server never reads the client's files. A load
spanning several relations, or one that needs routing or rejects,
begins an interactive transaction, declares a temp relation in it,
appends its text columns into it with an `Append` stream naming that
transaction, and then routes them with one ad hoc insert with
`rejects into` in the same transaction (§4.3).

### 4.2 Routing rows onto constructors

External rows carry no sums. Each shape of input has a route onto the
constructors of a relation-level sum:

| Input | Route |
| --- | --- |
| one file per constructor | one insert per constructor relation, in one statement or `transact` block, so the load is one change set |
| a tag column | read into a field of type `Tag T` and matched (below) |
| an Arrow dense union column | each child maps onto the constructor of the same name |
| a column holding values in the textual syntax | read at the sum's type (§1.1) |

A tag column is read into a field of the sum's tag type `Tag T` ([queries.md](queries.md#44-constructor-tags-on-the-wire)): the match over the result
names every constructor, so adding a constructor to the sum makes the
load fail to check until it routes the new one. A tag naming no
constructor at the file's version is a conversion failure and goes to
the rejects relation. Text already in the database, such as a staging
column, is parsed with `(Tag T).parse`, which reads at the caller's
version.

```adt
type IdentityRow = { id: Key identities, user: Key users, kind: Tag Identity,
                     email: Option Text, hash: Option Bytes,
                     gid: Option Text, sub: Option Text }

insert identities from (
  from r in read_csv("identities.csv", IdentityRow)
  select match r.kind
    | EmailPass -> (match r.email
                          | Some { value: e } -> (match r.hash
                                                     | Some { value: h } -> EmailPass { id: r.id, user: r.user, email: e, hash: h }
                                                     | None   -> reject "no hash")
                          | None   -> reject "no email")
    | Google    -> (match r.gid
                          | Some { value: g } -> Google { id: r.id, user: r.user, gid: g }
                          | None   -> reject "no gid")
    | Apple     -> (match r.sub
                          | Some { value: a } -> Apple { id: r.id, user: r.user, sub: a }
                          | None   -> reject "no sub"))
rejects into identity_rejects
```

`reject e` is an expression of any type, with `e : Text`. It may occur
only in the source query of an insert, and nowhere else. It routes the
input row to the insert's rejects relation with the message `e`
(§4.5).

### 4.3 Staging

To inspect and repair input before it is checked, a load lands rows in
a staging relation first, then moves them with one final insert as one
change set. A staging relation holds plain values, not references, so
nothing but its key is checked when rows land.

In ad hoc work the staging relation is usually a temp relation, which
needs no migration:

```text
temp relation NAME : T [ key ... ]
```

declares a relation, outside the log, of any named type or an
anonymous record type, with no constraint but its key; without a `key`
clause its whole row is its key. It belongs to the interactive
transaction (by its `TxnId`) or the one ad hoc text that declares it,
and is gone when that transaction or text ends. A session holds no
temp relation. A persistent staging relation is declared by a
migration like any relation.

The statements below run as one ad hoc text or in one interactive
transaction:

```adt
type IdentityText = { user: I64, kind: Text, email: Option Text, hash: Option Bytes,
                      gid: Option Text, sub: Option Text }
type IdentityStage = { line: Key identity_stage, user: I64, kind: Text,
                       email: Option Text, hash: Option Bytes,
                       gid: Option Text, sub: Option Text }
temp relation identity_stage : IdentityStage key line generate sequence
temp relation identity_rejects : RejectedRow key (load, line)

insert identity_stage from (
  from t in read_csv("identities.csv", IdentityText)
  select IdentityStage { user: t.user, kind: t.kind, email: t.email, hash: t.hash,
                         gid: t.gid, sub: t.sub })
update s in identity_stage where s.kind == "gogle" set IdentityStage { s with kind: "Google" }

insert identities from (
  from s in identity_stage
  select match (Tag Identity).parse(s.kind)
    | Some { value: EmailPass } -> (match s.email
        | Some { value: e } -> (match s.hash
            | Some { value: h } -> EmailPass { user: users.key(s.user), email: e, hash: h }
            | None              -> reject "no hash")
        | None              -> reject "no email")
    | Some { value: Google } -> (match s.gid
        | Some { value: g } -> Google { user: users.key(s.user), gid: g }
        | None              -> reject "no gid")
    | Some { value: Apple } -> (match s.sub
        | Some { value: a } -> Apple { user: users.key(s.user), sub: a }
        | None              -> reject "no sub")
    | None -> reject "unknown kind")
rejects into identity_rejects
```

`(Tag Identity).parse` returns `None` for text naming no constructor,
and that arm rejects the row. The final insert generates each row's
`id`, since `identities` generates its keys. A remote client fills
`identity_stage` with an `Append` stream naming the interactive
transaction (`txn`) instead of `read_csv`.

A constraint violation aborts the whole final insert (§4.5). To load
the rows that do not collide and keep the others for review, remove
the colliding rows from staging first, as ordinary ad hoc writes:

```adt
type IdentityDupe = { line: I64, user: I64, kind: Text,
                      email: Option Text, hash: Option Bytes,
                      gid: Option Text, sub: Option Text }
temp relation identity_dupes : IdentityDupe key line

insert identity_dupes from (
  from s in identity_stage, i in identities.EmailPass
  where s.email == Some { value: i.email }
  select IdentityDupe { line: value(key s), user: s.user, kind: s.kind,
                        email: s.email, hash: s.hash, gid: s.gid, sub: s.sub })
delete s in identity_stage
where exists i in identities.EmailPass where s.email == Some { value: i.email }
```

Rows that collide with each other inside the file are found the same
way, by a self-join of the staging relation.

### 4.4 Checking a large change set

A load's change set is checked by the rules of
[constraints.md](constraints.md#62-checking), at a cost independent of
how the rows arrived. A violation is
reported as the constraint's violation report
([constraints.md](constraints.md#7-checking-and-violation-reports)).

### 4.5 Rejects

Rows that fail to convert to the row type (§3.1) or that reach
`reject` go to the rejects relation named by the insert's `rejects
into` clause, a relation of the prelude type `RejectedRow`. Each
record's conversion is tried on its own, as
[writes.md §2.5](writes.md#25-rejected-rows) defines:

```adt
type RejectedRow = { load: I64, line: I64, source: Text, raw: Text, reason: Text }
relation identity_rejects : RejectedRow key (load, line)
```

The key holds no key of the relation itself, so one type serves every
rejects relation. `load` is an id the engine generates for each
inserting statement, so a rerun of a load never collides with an
earlier one; every reject of one statement has the same `load`. `source` is the file path, or the name of the relation
the rows were read from. `line` is the row's number, from 1, in the
order the source is read: records of a CSV or NDJSON file, rows of a
Parquet or Arrow IPC file, or the order of a source relation's key. `raw` is the record as the file held it, in the file's
own text where it has one and in the textual syntax otherwise.
`reason` is the conversion error, naming the column, or the `reject`
message.

The rejects are inserted in the same change set as the load. Every
other row is loaded. A constraint violation is not a reject: it aborts
the whole statement with `Rejected { reason: Violated {..} }`, or
`KeyViolation` for a collision on the relation's key, and no rejects
are kept. An insert with no `rejects into` clause fails, when any
record is rejected, with `Rejected { reason: BadInput {..} }`, which
carries the `source`, `line` and reason of one rejected record.

### 4.6 Durability of a load

A committed load is recovered after a crash and reaches every replica,
as any commit. The log written for a bulk load must not exceed about
one copy of the data the load stores.
A load into a relation a feed names also writes the logical change
records the feed reads (§5.2).

Rulings: interop/load/source, interop/load/append-api, interop/load/shape/per-ctor, interop/load/shape/tag-match, interop/load/shape/union-column, interop/load/shape/tagged-text, interop/load/shape/unrouted/tag-parse, interop/load/unit/staged, interop/load/check/set-at-a-time, interop/load/errors/reject-input, interop/load/wal/page-images, spec/writes/sequence-distinct/issued-and-held

## 5. Export and change streams

### 5.1 Export

`export` is a command ([catalog.md](catalog.md#91-commands)): it runs
a query in one read-only transaction and writes its result to a file
in one of the formats of §2. Through a server it needs the
`write_files` privilege and may write only inside the directories of
the open setting `export_roots`, which is empty by default; a path
outside them is refused with `Rejected { reason: Denied {..} }`. A
caller of the in-process API writes any path the process can.

```adt
type Via = Email { addr: Text } | Phone { num: Text }
type Contact = { name: Text, via: Via }
relation contacts : Contact key name

export (from i in identities.Google select i) to "google.parquet"
export (from c in contacts select c) to "contacts.csv" sums text
```

The format is given by the file's extension (`.csv`, `.ndjson`,
`.parquet`, `.arrow` for the IPC file format, `.arrows` for the stream
format). `sums text` writes value-level sums as text cells (§2.3); it
is accepted only for CSV. The file appears complete or not at all. A
result with a nested set field cannot be exported to CSV and is
refused with `Rejected { reason: Invalid {..} }`.

Exports state the schema version their constructor names are written
at:

- Parquet and Arrow IPC writers record the version line of §1.2 in the
  file's key-value metadata under the key `adtb.text`.
- An NDJSON export's first line is a header object:
  `{"adt": {"edition": "2026-10", "pins": 42, "hash": "c7f2..."}}`.
- A CSV export writes nothing beyond the header row of column names.

A reader of a file with a version (metadata or NDJSON header) reads its
names at that version and translates them as for a text document
(§1.2); the NDJSON header line is not a row. A file with no version is
read at the reader's version. A CSV file whose names are stale is read
by giving its version to the reader:

```adt fragment
read_csv("contacts-old.csv", Contact) pins 41 (b3e0...)
```

### 5.2 Change streams

A change stream delivers committed change sets to an external
consumer. A consumer is registered like a bundle: a feed is declared
in a bundle, which pins the schema version the stream is encoded at.

```adt
bundle search-sync-1.0 pins 12 (c41e...) edition "2026-10"
feed identity_changes of identities.(Google | Apple)
```

The stream of a feed holds, in commit order, one event for each
committed transaction that changed a constructor relation the feed
names. An event carries the commit's position and, for each named
constructor relation it changed, the rows it deleted and the rows it
inserted. Rows are encoded at the bundle's pinned version, translated
as for any pinned reader. The protocol is defined in
[wire.md](wire.md#9-feeds).

A retag is a delete from the old constructor relation and an insert
into the new one, in the same event. A feed naming only one of the two
sees only that half.

Because the feed is part of a registered bundle, it is a reader of the
schema like any other ([evolution.md](evolution.md#81-enabling-a-constructor)).
A feed makes `enable r.C` refused only when it names `r` whole, as
`feed f of r`, at a version where `C` does not exist, and only while
its bundle is live, until it is retired, superseded or expired. A feed of
named constructor relations, such as `identities.(Google | Apple)`,
never carries a constructor it does not name and never blocks. A
bundle whose header says `hide new constructors` never blocks
`enable r.C`, and its feed does not carry rows of constructors its version
lacks.
Volatile relations cannot be named by a feed.

### 5.3 Retention

The log is retained for feeds for the `FeedRetention` file setting
([catalog.md](catalog.md#10-configuration)) and no longer. A consumer
resuming from a position older than that is refused with `TooOld`
([wire.md](wire.md#9-feeds)) and must resnapshot: subscribe again from
a snapshot, which first delivers every row of the feed's constructor
relations at one snapshot and then every change set committed after
it, with no gap and no overlap. A stalled
consumer never holds back the log beyond the retention period.

Rulings: interop/formats/csv, interop/formats/ndjson, interop/formats/parquet, interop/formats/arrow-ipc, repl/cdc/change-stream, repl/cdc/registered-consumer, repl/cdc/retag/ctor-pair, repl/cdc/retention/bounded, repl/cdc/retention/resnapshot

## 6. Dump and restore

### 6.1 A logical dump

A dump is a logical copy of a database: it is independent of the page
format and engine release, so it moves data between releases and can
be reviewed. A physical copy is a backup (§7).

```sh
$ adt dump app.adt app.dump
$ adt restore app.dump new.adt
```

`adt dump` and `adt restore` wrap the commands `dump` and `restore`
([catalog.md](catalog.md#91-commands)). A dump needs `maintain` and
`read` on every relation it dumps; its `security` entry is written
only for a holder of `schema`.

A dump reads the database at one snapshot. It is a directory holding:

| Entry | Content |
| --- | --- |
| `manifest` | the edition, the engine release, the dump format, the schema version with its hash, the catalog version, the snapshot's position, whether the dump is encrypted, and a BLAKE3-256 checksum of every other entry |
| `log` | the whole operator log, every migration in order, in its canonical text |
| `blobs/` | every Wasm module, extension package and tzdata release the log names, each under its BLAKE3-256 hash |
| `ids` | for every catalog object other than a constructor, the key the database assigned it |
| `generators` | the next value of every key generator |
| `bundles/` | every registered bundle with its deployment state: owner, pinned and catalog versions, edition, lease and state |
| `security` | the deployment state of security: the rows of `catalog::roles`, the credential verifiers of `catalog::credentials` (never plaintext secrets), the rows of `catalog::grants` of kind `Deployment` (capability grants `adhoc`, `assert`, `schema`, `monitor`, `cancel`, `maintain`, `configure`, `read_files`, `write_files`, bundle call grants, and grants on `catalog::` and `sys::` relations), and the peer mappings of `catalog::peer_mappings` |
| `settings` | the rows of `catalog::settings` |
| `requests` | the request marks and recorded replies of `sys::request_marks` |
| `audit` | `catalog::audit` with its ledger |
| `extensions` | each installed extension by name, version and content hash |
| `data/R.C.arrows` | for each constructor relation `R.C`, its rows as an Arrow IPC stream in the mapping of §2.2 |
| `data/R.C.txt` | optionally, the same rows as a text document (§1.2) |

Relation and constructor-set grants and `grant call fn` are log
operators, rows of `catalog::grants` of kind `Log`, so the `log`
carries them and a restore reinstates them by replay. A restore writes
the roles of `security` before it replays the log; a log grant naming
a role the dump does not hold, as in a dump written without `security`,
is kept dormant and allows nothing, and the restore reports every
dormant grant ([security.md](security.md#21-roles)). The text form of
`data/` is for review; a restore reads the Arrow form.

A dump is written in plaintext unless it is written with `--encrypt
KEY` (`dump to "path" encrypt key` as a command), which encrypts every
entry but the manifest's version fields under that key; a restore of
an encrypted dump needs the key.

Not dumped, and lost by a dump and restore:

| Not dumped | What clients see after a restore |
| --- | --- |
| commit history and retained old versions | past reads (`at commit`, `at instant`) at commits before the restore fail with `HistoryUnavailable` |
| feed positions and checkpoints | feed consumers resnapshot (§5.3) |
| stored plans | every registered query is replanned at restore |
| statistics and other volatile relations | statistics start empty and are rebuilt online |

### 6.2 What a restore preserves

A restore reproduces the dumped database exactly as clients see it:

- equal relations, row for row;
- the same schema version and hash, with the whole log, so bundles
  pinned to earlier versions are still translated;
- the same constructor ids, which replaying the log reproduces, and
  the same keys of every other catalog object, taken from `ids`;
- the same key values, including generated ones, with every generator
  resuming at its dumped next value;
- every bundle registered at its pinned version, with its owner, lease
  and state;
- the same roles, grants, credential verifiers and peer mappings, so
  every principal authenticates and is authorised as before, except
  that a log grant whose role the dump lacks is dormant and reported;
- the same file settings, and the audit relation with its ledger, whose
  head digest is unchanged;
- every extension, Wasm module and tzdata release, from `blobs/`, each
  checked against the hash the log records;
- the request marks and recorded replies, so a client resending a
  request id gets its recorded reply and is not run twice.

A restore needs an engine release that reads the dump's format and
whose catalog log extends the dump's catalog version; any other
release refuses the dump before writing anything.

A deployed client keeps working against the restored database with no
redeploy. Host functions must be supplied by the program that opens
the restored file.

### 6.3 Validation

A restore writes into a new file that is not a database until the
restore completes. It replays the log's schema operators onto empty
relations: `script` operators are skipped, since the dumped rows are
their result, and `install` and `adopt tzdb` take their bytes from
`blobs/`. It then writes
every dumped row without checking it, builds every index from sorted
runs, checking uniqueness as it builds, then runs every constraint's
query once over the whole database. If any uniqueness or constraint
query finds a violation, the restore fails with the violation reports
and the file is not usable. No reader ever sees a state the
constraints have not checked.

Rulings: interop/dump/logical, interop/dump/schema/op-log, interop/dump/data/arrow-ipc, interop/dump/data/text-rows, interop/dump/roundtrip/same-ids, interop/dump/scope/bundles, interop/dump/scope/extensions, interop/dump/restore/offline-validate

## 7. Backup and point-in-time restore

This section states what an operator can rely on.

### 7.1 Backup

A backup is a physical copy: a copy of the database file taken while
the database serves calls, plus the log from the copy's start position
to its end position. Restoring a backup replays that log over the copy
and yields the database as committed at the end position.

An incremental backup copies only the pages changed since a previous
backup's start position, and is restored together with the chain of
backups it builds on.

```sh
$ adt backup app.adt s3://backups/app
$ adt backup app.adt s3://backups/app --incremental
$ adt restore s3://backups/app restored.adt
```

`adt backup` and `adt restore` wrap the commands `backup` and
`restore` ([catalog.md](catalog.md#91-commands)). `adt restore` reads
either a dump (§6) or a backup; its manifest says which.

Every backup carries a manifest stating the engine release, format,
schema version, start and end positions, the backup it is
incremental to, and a checksum of each part. A backup is proven only
by a test restore that rebuilds the database and runs the full
verification of [catalog.md](catalog.md#111-verification) on it; the
tool records the result in the manifest. A backup copies encrypted
pages as they are stored ([security.md](security.md#95-backups-and-dumps)).

### 7.2 Point-in-time restore

With a continuous archive of the log, a restore replays a backup and
the archived log up to a chosen point:

- a commit position `Position { epoch, lsn }`, such as one returned
  by a commit, written `epoch/lsn` on the command line;
- an instant, through the recorded map from commit positions to wall
  time;
- a schema version: the state just before a given migration applied.

```sh
$ adt restore s3://backups/app restored.adt --before-version 12
$ adt restore s3://backups/app restored.adt --to-time 2026-10-24T13:02:00Z
$ adt restore s3://backups/app restored.adt --to-position 3/81234
```

The archive destination is an open setting
([catalog.md](catalog.md#103-open-settings)).

Rulings: repl/backup/base-plus-log, repl/backup/incremental, repl/verify/manifest, repl/verify/restore-check, repl/pitr/log-archive, repl/pitr/op-log-point

## 8. Migrating a SQL database

### 8.1 The tool

There is no SQL surface, and the engine never parses SQL. A separate
offline tool reads a SQL database's catalog and data and proposes a
schema and a load in the notation of this spec. The user edits and
confirms the proposal; the tool then runs the load.

```sh
$ adt import-sql postgres://old/app --propose > migrations/0001
$ adt import-sql postgres://old/app --load migrations/0001 app.adt
```

The proposal is a first migration of ordinary operators and a load
script of ordinary inserts (§4); nothing the tool emits needs it at
run time.

### 8.2 Finding sums

The tool finds candidate sums in three ways, and applies none without
confirmation:

- The user declares a sum: a table, its tag column, and each
  constructor's columns.

  ```sh
  $ adt import-sql postgres://old/app --propose --sum vehicles.type=Car:wheels,Boat:sail_area
  ```

- A CHECK constraint relating a tag column to the nullness of other
  columns states a sum outright, and is read as one.
- The data is profiled: rows are grouped by tag value and by which
  columns are non-null, and each tag whose rows share one pattern is
  proposed as a constructor with those columns.

### 8.3 Patterns

| Source pattern | Proposal |
| --- | --- |
| a nullable column | an `Option` field |
| a type column with per-type nullable columns (single-table inheritance) | a relation-level sum, one constructor per type |
| inheritance children of one parent table | one sum, one constructor per child |
| a type column with an id column (a polymorphic reference) | a relation-level sum with one constructor per target, each with its own reference |
| several nullable references with an exactly-one check (an exclusive arc) | the same sum as a polymorphic reference |
| a JSON column whose objects carry a type field | a value-level sum, one constructor per type value, with the fields found |

```adt
-- source: vehicles(id, type, wheels NULL, sail_area NULL)
type Vehicle { id: Key vehicles } = Car { wheels: I32 } | Boat { sail_area: F64 }
relation vehicles : Vehicle key id generate sequence

-- source: employees(id, name), products(id, name)
type Employee = { id: Key employees, name: Text }
type Product = { id: Key products, name: Text }
relation employees : Employee key id generate sequence
relation products : Product key id generate sequence

-- source: pictures(id, imageable_type, imageable_id)
type Picture { id: Key pictures } = OfEmployee { owner: Key employees }
                                  | OfProduct  { owner: Key products }
relation pictures : Picture key id generate sequence
```

### 8.4 NULL

A SQL NULL becomes `None`, and `None` is an ordinary value: two `None`
values are equal. A SQL unique over a nullable column admits many
NULLs, so the tool translates it into a uniqueness over the `Some`
values only:

```adt
type Member = { id: Key members, email: Option Text }
relation members : Member
  key id generate sequence
  index (email)
constraint members_email_unique =
  from a in members, b in members
  where a.email == b.email and a.email != None and key a != key b
  select { a: key a, b: key b }
```

The tool reports every source constraint whose meaning changes under
the mapping, with what it proposes:

```text
members.email UNIQUE, nullable   -> unique over Some values only (translated)
orders.qty    CHECK (qty > 0)    -> passes on NULL in SQL; choose an Option match or a default
```

### 8.5 Dirty data

Source rows may violate constraints the old database never enforced.
For each violation class the tool:

- emits a repair query against the source, for the user to run before
  migrating;
- offers to load the violating rows into a quarantine relation typed by
  the source row, keyed by the source key, for repair after migrating.

The clean rows are committed only if no clean row depends on a
quarantined one; otherwise the load fails with the violation report,
which names the dependent rows.

Rulings: interop/migrate/tool, interop/migrate/infer/declared, interop/migrate/infer/from-checks, interop/migrate/infer/profile, interop/migrate/patterns/nullable-option, interop/migrate/patterns/sti, interop/migrate/patterns/pg-inherit, interop/migrate/patterns/polymorphic, interop/migrate/patterns/exclusive-arc, interop/migrate/patterns/jsonb-tagged, interop/migrate/null/translate-unique, interop/migrate/null/report, interop/migrate/dirty/quarantine, interop/migrate/dirty/fix-script

## 9. User-defined functions

### 9.1 Kinds

Besides functions written in the query language
([derived.md](derived.md#1-views)), a schema function may have its
body in:

- a WebAssembly module, stored in the catalog;
- the embedding program, as a Rust function supplied when the file is
  opened in process.

From version 18:

```adt
-- 0019
module auth
define fn geohash (lat: F64, lon: F64, precision: I32) -> Text =
  wasm "geo.wasm" export "geohash"
define fn slugify (t: Text) -> Text = host "slugify" version "1.0" deterministic
```

The signature is declared in the schema and checked like any function
signature. A Wasm module's bytes are read when the migration is
authored and stored in `catalog::wasm_modules`. The `define fn`
operator's value holds their BLAKE3-256 hash, so the version hash
covers them, and the log, dumps and replicas carry the bytes.

### 9.2 WebAssembly functions

A module defining a function must import nothing. Defining a function
from a module with imports is refused with `Rejected { reason:
SchemaRefused {..} }`. A Wasm function is evaluated with NaN results
canonicalized and with deterministic fuel metering, so it is a pure
function of its arguments: every engine, replica and simulation
computes the same result. A trap, or exhausting its fuel or its memory
bound, is a fault of the calling statement.

### 9.3 Host functions

A host function's body is registered by the embedding program with
the name and version the schema declares
([embedding.md](embedding.md#10-host-functions)). The declaration states
whether it is `deterministic`; the engine trusts the declaration.

Only deterministic functions may appear in a key, an index
expression, a constraint, a row check, a stored view, a default or a
computed field. A nondeterministic host function may appear only in
queries and in the values of writes; each call in the query's meaning
is evaluated exactly once.

An embedded open binds host functions before it applies the shipped
migrations ([embedding.md](embedding.md)). A host function registered
at a version other than the one the catalog records fails the open
with `HostFnMismatch`, unless the migrations the open applies contain
the `redefine fn` that moves the catalog to that version
([embedding.md](embedding.md#102-missing-and-mismatched-functions)).
Such an open fails with `HostFnMismatch` naming the function when a
staged job or index build of a migration before that `redefine fn`
reaches the function; otherwise the open proceeds.
The open never writes a redefinition on its own. An open by an older
program, whose host function the database has since redefined, fails
with `Rejected { reason: DowngradeRefused {..} }` and changes
nothing.

A file opened without a host function it declares fails to open when
an index, constraint, stored view or key generator reaches the
function; otherwise it opens, and a call that reaches the function
fails with `Fatal { reason: MissingFunction {..} }`
([embedding.md](embedding.md#10-host-functions)).

### 9.4 The calling convention

A Wasm or host function is called once per batch of rows, not once per
row. Its arguments arrive as one Arrow array per parameter, all of one
length, in the mapping of §2.2 (value-level sums as dense unions); it
returns one array of the result type and the same length.

A Wasm module must export:

- `memory`, its linear memory;
- `adt_alloc(len: i32) -> i32`, which returns the offset of `len`
  fresh bytes aligned to 16, or 0 when it cannot;
- `adt_call(fn_id: i32, args_ptr: i32, args_len: i32) -> i32`.

The module carries a custom section named `adt.functions` holding its
function names, one per line in UTF-8; the `export` clause of `define
fn` names one of them, and `fn_id` is that name's position, from 0.
`args_ptr` points to `args_len` pairs of `ArrowSchema` and `ArrowArray` structs of the Arrow C data
interface, one pair per parameter, laid out with 32-bit pointers, all
in linear memory placed by `adt_alloc`. `adt_call` returns the offset
of one such pair holding the result array, or 0 for a fault of the
calling statement. The engine copies the result out before the next
call and never calls the structs' release callbacks; the module may
reuse its memory after `adt_call` returns. A module missing any of
these exports, or whose `adt.functions` lacks the named function, is
refused when the function is defined.

### 9.5 Function identity

A function's identity includes a hash of its code: the BLAKE3-256 hash
of the module bytes for Wasm, of the declared version for a host
function, of the body for a query-language function.
`catalog::functions` records it. Changing the code is the schema
operator `redefine fn`, which rebuilds every index and stored view
whose expression calls the function, directly or through another
function, and rechecks every constraint and registered query that
calls it, as a staged job
([evolution.md](evolution.md#34-staged-changes)).

Which code runs:

- A pinned bundle evaluates the definitions of the version it pins.
- Indexes, constraints and stored views always use the current code.
  A pinned bundle that reads a stored view redefined since its pin
  evaluates the body of its own version, not the stored rows
  ([derived.md](derived.md#26-schema-changes)).
- An expression index never changes a query's result: the query's
  expression is evaluated with the code of the query's version, even
  where an index over the same expression holds values computed with
  other code.
- A host function has one code in a process. Redefining it is refused
  while a live bundle pins a version at which it has another code
  identity.

From version 19:

```adt
-- 0020
module auth
redefine fn geohash (lat: F64, lon: F64, precision: I32) -> Text =
  wasm "geo-2.wasm" export "geohash"
```

### 9.6 Extensions

An extension is a module of definitions (types, functions,
aggregates) together with the Wasm code they use, packaged as one
file. Installing it is a schema operator, so its definitions and code
become catalog rows. Its operator value holds the package's
BLAKE3-256 hash, so the version hash covers it, and the log and dumps
carry its bytes. From
version 20:

```adt fragment
-- 0021
module auth
install geo "1.2" from "geo-1.2.adtx"
```

Names it defines are qualified by its module, `geo::cell`. An
extension adds nothing a schema could not define itself: no new base
types, index kinds or native code.

Rulings: interop/udf/wasm, interop/udf/host-fn, interop/udf/purity/wasm-det, interop/udf/purity/declared, interop/udf/abi/arrow, interop/udf/identity/content-hash, interop/ext-pkg/catalog-module

## 10. User-defined aggregates

An aggregate is declared by a state type, its initial value, a step
that adds one input, a merge that combines two states, and a finish
([syntax.md](syntax.md#59-user-defined-aggregates)). A bundle declares
one directly; a schema adds one with `define` in a migration, as for a
schema function. It is called like the built-in aggregates, over
bindings ([queries.md](queries.md#10-aggregates)):

```adt
aggregate mean (x: F64) -> F64 =
  state { s: F64, n: F64 } init { s: 0.0, n: 0.0 }
  step (st, x) -> { s: st.s + x, n: st.n + 1.0 }
  merge (a, b) -> { s: a.s + b.s, n: a.n + b.n }
  finish st -> st.s / st.n

type Sale = { id: Key sales, region: Text, total: F64 }
relation sales : Sale key id generate sequence

from o in sales group o by o.region into g
select { region: g.key, mean: mean(x.total for x in g) }
```

The engine may split the input into any number of parts, fold each
from `init` with `step` on any worker, and combine the partial states
with `merge` in any order before one `finish`. The aggregate's author
must make `merge` associative and commutative with `init` as its
identity, and folding any split of the input then merging must give
the state folding all of it would; the engine does not check this.

Over a range that is a nonempty group the result has the declared
type. Over a range that may be empty the result is an `Option` of it,
`None` for an empty range, without calling `finish`.

The state is a value of a record or declared type, so it holds no
collection.
`step`, `merge` and `finish` are query-language expressions and may
call Wasm functions; an aggregate is deterministic when every function
it calls is.

Rulings: interop/uda/fold-decl

## 11. Scalars, indexes and search

### 11.1 No new scalar types

Every type is a sum, a record or a newtype over the base types of
[model.md](model.md#3-base-types). No extension adds a scalar type, so
the key encoding, order, equality and hashing of every type are
derived from the base types. A point is a record; an address range is
a newtype over `Bytes`.

```adt
type Point = { x: F64, y: F64 }
type Cidr = new Bytes
```

### 11.2 Index kinds

Index kinds are fixed: the keys, uniques, indexes and interval indexes
of [model.md](model.md#7-keys) and [constraints.md](constraints.md#21-keys-uniques-and-references),
each declared over a constructor set. New index kinds cannot be added.
What each form lets a query do:

| Form | Lets a query |
| --- | --- |
| key, unique | find the rows holding given values of its fields |
| index | find the rows holding given values of its fields or of a leading part of them, and the rows within a range of its fields in their order |
| interval index | find the rows whose interval field overlaps a given span, among those holding given values of its leading fields |

These are the indexes that bound a constraint's or a stored view's
incremental check ([constraints.md §5](constraints.md#5-admission)).
The structure behind a form is not observable.

Special indexes are derived: an index over an expression of the row
([derived.md](derived.md#52-expression-indexes)). When the expression's
type is a set, each of its elements is one index entry for the row.
A derived index is maintained from each change set like any index, and
has the same transactional, logging and recovery behaviour.

### 11.3 Full-text search

Full-text search is a derived index over a tokenizer: a function from
text to the set of its terms, written in the query language or Wasm.

```adt
type Doc { id: Key docs, title: Text } = Article { body: Text } | Note { body: Text }
                                       | Link { url: Text }
relation docs : Doc
  key id generate sequence
  index (words(body)) in (Article | Note)

fn words (t: Text) -> Set ({ term: Text, pos: I32 }) =
  wasm "unicode-words.wasm" export "words"

from d in docs.(Article | Note)
where exists w in words(d.body) where w.term == "sum"
select d
```

A predicate over the indexed expression finds its rows through the
index, as for any index. Phrase search and ranking are queries over the same term
sets.

### 11.4 Vectors

An embedding is a newtype over `Bytes` whose domain fixes its length.
Distance functions are ordinary functions over such values. Search is
exact, by a scan:

```adt
type Emb768 = new Bytes where bytes_len(self) == 3072
type Chunk { id: Key chunks, embedding: Emb768 } = Para { text: Text } | Code { source: Text }
relation chunks : Chunk key id generate sequence
fn cosine (a: Emb768, b: Emb768) -> F64 = wasm "vectors.wasm" export "cosine"

query nearest (q: Emb768) -> Set Chunk =
  top 10 by (cosine(c.embedding, q) desc, key c) (from c in chunks.(Para | Code) select c)
```

There is no approximate nearest-neighbour index. Approximate search
runs in an external system fed by a change stream (§5.2).

### 11.5 Out of scope

Spatial indexes are out of scope: spatial data is indexed by cells
(geohash or S2 cell ids) computed by a function and used as ordinary
keys of a derived index. JSON is a format only (§2.5).

```adt
fn geohash (lat: F64, lon: F64, precision: I32) -> Text =
  wasm "geo.wasm" export "geohash"
type Point = { x: F64, y: F64 }
type Store = { id: Key stores, name: Text, location: Point }
relation stores : Store
  key id generate sequence
  index (geohash(location.y, location.x, 7))
```

Rulings: interop/scalar/newtype-only, interop/index-am/fixed, spec/index/kinds/declared-capabilities, interop/index-am/derived, interop/fts/derived, interop/vector/dim-type/newtype-bytes, interop/vector/scan-only, interop/spatial/out
