# Errors

Every failure the engine reports is a value of one sum, `Error`, with
three constructors. The constructor tells you what to do; the reason
inside it tells you why.

| Constructor | What it means | What to do |
| --- | --- | --- |
| `Retry` | Nothing was applied, and the same request may succeed if you run it again. | Run it again, usually at once. |
| `Rejected` | Nothing was applied, and the same request fails again until something changes: the text, the data, the schema or the program. | Fix the cause, or report it to whoever can. |
| `Fatal` | The database cannot serve the request. | Stop and have an operator look. |

Opening a database can also fail before any call runs, with an open
error of its own; those are at the [end of this page](#open-errors).

This page lists every error you can meet in this engine, what causes
it and what to do about it. The full list of reasons, including those of the server
protocol and of replicas, is in
[the specification](../../spec/catalog.md#4-error-sums).

## How errors look

The shell prints the message of an error on standard error and, when
it runs a script or `-c` text, exits with status 1. The examples on
this page use the shelter schema of the [Rust API](rust-api.md) page,
as two migrations:

```adt migrations/0001.adt
module shelter
add module shelter
add type Pet { name: Text } = Cat { lives: I64 } | Dog { good: Bool }
add relation pets : Pet key name
```

```adt migrations/0002.adt
module shelter
add type Adoption = { pet: Key pets, by: Text }
add relation adoptions : Adoption key pet
define fn shout (t: Text) -> Text = host "shout" version "1.0" deterministic
```

<!-- exit 1 -->
```console
$ adtb --create --schema migrations/0001.adt --schema migrations/0002.adt shelter.db
adtb> insert pets.Cat { name: "tom", lives: 9 }
adtb> insert pets.Cat { name: "tom", lives: 3 }
`shelter::pets` already holds a row with key `pets.key("tom")`
adtb> \q
```

In Rust, a call returns `adtb::Error`, or for a call through a bundle,
the bundle's generated `Error`; both have the variants `Retry`,
`Rejected` and `Fatal`, each holding its reason as an enum:
`RetryReason`, `RejectReason` or `FatalReason`. Printing an error with
`{}` gives the message the shell prints. `value()` gives the error as
a value of the sum, in the textual syntax, which is what to log when a
program has to tell reasons apart later. `Retry` messages end in
`; retry`, and `Fatal` ones start with `fatal:`.

```rust
use adtb::{CallOptions, CreateOptions, Error, Log, OpenOptions, RejectReason};

fn main() {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
    ]);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let insert = "insert pets.Cat { name: \"tom\", lives: 9 }";
    let opts = CallOptions::default();
    db.query_text(insert, &[], &opts).expect("the first tom");
    let Err(e) = db.query_text(insert, &[], &opts) else {
        panic!("a second tom");
    };
    println!("{e}");
    println!("{}", e.value());
    if let Error::Rejected {
        reason: RejectReason::KeyViolation { relation, key },
    } = e
    {
        println!("{relation} holds {key}");
    }
}
```

```text
`shelter::pets` already holds a row with key `pets.key("tom")`
Rejected { reason: KeyViolation { relation: "shelter::pets", key: "pets.key(\"tom\")" } }
shelter::pets holds pets.key("tom")
```

## Retry

### Serialization

The transaction conflicted with a concurrent one, and running both
would not have been the same as running them one after the other, so
the engine aborted it. Nothing it did was applied. Run it again: in a
fresh transaction it sees the other's writes. Outside an interactive
transaction, the engine reruns a registered transaction, and ad hoc
text that writes, before the error reaches you, up to the retry limit;
`transact` reruns its closure the same way. An interactive transaction
is never rerun for you.

```rust
use adtb::{CreateOptions, Log, OpenOptions, TxnCallOptions, TxnOptions};

fn main() -> Result<(), adtb::Error> {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
    ]);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let opts = TxnCallOptions::default();
    let mut first = db.begin(&TxnOptions::default())?;
    let mut second = db.begin(&TxnOptions::default())?;
    first.query_text("count(p for p in pets)", &[], &opts)?;
    second.query_text("count(p for p in pets)", &[], &opts)?;
    first.query_text("insert pets.Cat { name: \"tom\", lives: 9 }", &[], &opts)?;
    second.query_text("insert pets.Dog { name: \"rex\", good: true }", &[], &opts)?;
    first.commit(None)?;
    let id = second.id().0.to_string();
    if let Err(e) = second.commit(None) {
        println!("{}", e.to_string().replace(&id, "N"));
        println!("{}", e.is_serialization());
    }
    Ok(())
}
```

```text
transaction N conflicts with a concurrent transaction; retry
true
```

Each transaction counted the pets and then added one, so each read
what the other wrote; whichever commits second fails. `N` is the
transaction's id, `id()`. `is_serialization` tells a serialization
failure apart from every other error.

### FileChanged

A file a query scans changed between two reads of it in one
transaction. Run the transaction again once the file has stopped
changing.

## Rejected: the text

### Invalid

The text does not parse or does not type check, a command does not
apply, or an argument is outside its parameter's type. The message
says what is wrong:

<!-- exit 1 -->
```console
$ adtb --create --schema migrations/0001.adt --schema migrations/0002.adt text.db
adtb> from c in pets.Cat selec c
expected `select`, found `selec`
adtb> from c in pets.Cat select c.age
Pet.Cat has no field `age`
adtb> from p in pets where p.name == who select p
`who` is a parameter of this text, but the call supplies no value for it
adtb> insert pets { name: "kit", lives: 3 }
a single row is inserted into one constructor of Pet: name it, or insert constructor values with `from (...)`
adtb> print schema
`print schema` does not run in this engine yet
adtb> \q
```

Fix the text. A command or operator the engine does not run yet is
`Invalid` too, with "does not run in this engine yet".

## Rejected: the data

These come from writes that would leave the database breaking one of
its own rules. The whole statement, or the whole transaction it ends,
applies nothing.

### KeyViolation

An insert gives a key that another row already holds, or two rows of
one change set share a key. The message names the relation and the
key. Change the key, or update the row that holds it instead of
inserting another.

### Violated

A reference, a unique, a check or a constraint does not hold at the
end of the statement. The message names the constraint, as
`MODULE::NAME`, and how many rows violate it. A reference is the
constraint `MODULE::RELATION.TYPE.FIELD`:

<!-- exit 1 -->
```console
$ adtb --create --schema migrations/0001.adt --schema migrations/0002.adt refs.db
adtb> insert pets.Cat { name: "tom", lives: 9 }
adtb> insert adoptions { pet: pets.key("felix"), by: "ann" }
constraint `shelter::adoptions.Adoption.pet` is violated by 1 row
adtb> insert adoptions { pet: pets.key("tom"), by: "ann" }
adtb> delete p in pets where p.name == "tom"
constraint `shelter::adoptions.Adoption.pet` is violated by 1 row
adtb> \q
```

Fix the data the write produces: here, adopt a pet that exists, or
delete the adoption before the pet. In Rust, the reason's `report`
holds the violating rows, as values of the constraint's result.
[Writing constraints](../guides/writing-constraints.md) explains which
rules the engine checks and how each is named.

### UserError

A write evaluated `fail e`. The message is the value of `e`, so it is
whatever the text or the bundle says it is:

<!-- exit 1 -->
```console
$ adtb --create --schema migrations/0001.adt --schema migrations/0002.adt fail.db -c 'transact fail "the shelter is full"'
the shelter is full
```

### NotOpen

An insert into a constructor that is not open for writes: one a
migration added and no later migration enabled, or one deprecated
since. The message names the constructor and its stage:
"constructor `pets.Bird` is Declared, not open for writes". Ship the
migration that enables it, as
[the migrations tutorial](../tutorial/7-migrations.md) does, or stop
writing a deprecated one.

### KeyChanged

An update gave a row another key. A row's key is its identity: delete
the row and insert one with the new key instead. The checker refuses
most such updates before they run, as `Invalid`.

### RowTooLarge

A row's stored form is larger than the engine's row limit. The message
names the relation, the constructor and both sizes in bytes. Store the
large part in a relation of its own, split into rows.

## Rejected: evaluation

### Faulted

A statement evaluated an expression that has no value, such as a
division by zero or an integer that overflows. The message names the
fault, a constructor of the prelude sum `Fault`:

<!-- exit 1 -->
```console
$ adtb --create --schema migrations/0001.adt --schema migrations/0002.adt faults.db
adtb> insert pets.Cat { name: "tom", lives: 9 }
adtb> from c in pets.Cat select c.lives / (c.lives - 9)
fault `DivisionByZero`
adtb> from c in pets.Cat select c.lives + 9223372036854775807
fault `ArithmeticOverflow`
adtb> \q
```

Guard the expression, or use the checked form that returns an
`Option`; [the prelude](prelude.md#fault) lists every fault and the
functions that raise it. A host function that returns an error or
panics faults with `FunctionFailed`.

### BadInput

A row of a file a query scans does not convert to the row type. The
message is `SOURCE:LINE: DETAIL`. Fix the file, or give the scan a
rejects relation; [Files and CSV scans](../guides/files-and-csv.md)
shows both.

### StatementLimit

A statement ran past a limit: its time limit, the memory it may use
even after spilling to disk, or the rounds a fixpoint may take. The
message names the limit. A fixpoint that does not converge within the
`fix_rounds` the database was opened with stops this way:

```rust
use adtb::{CallOptions, CreateOptions, Log, OpenOptions};

fn main() {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
    ]);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        fix_rounds: Some(10),
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let text = "fix r = [1] union (from x in r select x + 1) in count(x for x in r)";
    if let Err(e) = db.query_text(text, &[], &CallOptions::default()) {
        println!("{e}");
    }
}
```

```text
the statement exceeded its limit of 10 fixpoint rounds
```

Rewrite a fixpoint that grows without end so that it converges. For
the other limits, make the statement cheaper, or raise the limit: a
call's `limit`, the `StatementLimit` setting, or the memory the
database was opened with. The engine does not rerun a statement that
stopped at a limit.

### Cancelled and DeadlineExceeded

The call's cancellation token was cancelled, or its deadline passed,
before it finished: "the call was cancelled", "the call's deadline
passed". The call has no effect. [The Rust API](rust-api.md#cancellation-and-deadlines)
shows both.

### HistoryUnavailable

An `at commit` or `at instant` block names a state the engine no
longer keeps: older than the `RetentionWindow` setting allows, or
later than the reading transaction's snapshot. The message names the
oldest position kept, as "no history is kept before { epoch: E, lsn:
N }". Read a later position, or raise `RetentionWindow` before the
commits you will want to read. This engine also refuses a read of a
relation that a later migration rebuilt, such as by adding a field to
its type. [Reading the past](../guides/reading-the-past.md) shows the
blocks and the window.

### Internal

The engine broke a promise its own checker made about the text: a bug
in the engine. The statement applies nothing, the transaction stays
open, and the engine keeps serving. Report it with the text that
caused it.

## Rejected: bundles

These come from calling a bundle, and from opening a database with
one. The examples register a cut-down copy of the bundle of the
[Rust API](rust-api.md) page from its file, without generated code:

```adt shelter-1.0.adb
bundle shelter-1.0 pins 2 (c2d244dc1b156f7f2e2cb7a03cd67d2c1aa27c156b88cde97567011c7fba4ad8) edition "2026-10"
import shelter
import fn shout
query names () -> Set Text = from p in pets select p.name
query shouted () -> Set Text = from p in pets select shout(p.name)
```

### UnknownBundle and UnknownQuery

No bundle of that name is registered by this open, or the bundle has
no callable of that ordinal. Register the bundle when you open the
database, or call by a name the bundle declares.

```rust
use adtb::{CallOptions, CreateOptions, Log, OpenOptions, Registration};

fn main() -> Result<(), adtb::Error> {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
    ]);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![Registration::bundle(include_str!("../shelter-1.0.adb"))?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    if let Err(e) = db.registered("shelter-2.0") {
        println!("{e}");
    }
    let app = db.registered("shelter-1.0")?;
    if let Err(e) = app.call(9, &[], &CallOptions::default()) {
        println!("{e}");
    }
    Ok(())
}
```

```text
no bundle `shelter-2.0`
no such query
```

### Untranslatable

A bundle pinned at an earlier version than the database's runs each
call translated from its pinned version to the current one. When the
database opens, a bundle with a query or transaction that has no
translation is refused registration, and the message names the query
and the operator it cannot cross. A query that reads a field a later
migration drops has none. Change the query, or pin the bundle at the
version the log ends at and regenerate its code.

```rust
use adtb::{CreateOptions, Log, OpenOptions, Registration};

const OLD: &str = "bundle shelter-1.0 pins 2 (c2d244dc1b156f7f2e2cb7a03cd67d2c1aa27c156b88cde97567011c7fba4ad8) edition \"2026-10\"
import shelter
query adopters () -> Set Text = from a in adoptions select a.by
";

const DROP_BY: &str = "module shelter
drop field Adoption.by
";

fn main() -> Result<(), adtb::Error> {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
        DROP_BY,
    ]);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![Registration::bundle(OLD)?],
        ..OpenOptions::default()
    };
    if let Err(e) = adtb::open("shelter.db", &log, &options) {
        println!("{e}");
    }
    Ok(())
}
```

```text
bundle shelter-1.0 is refused: query `shelter-1.0::adopters` cannot be translated: across `drop field Adoption.by`: it reads field `by` of `Adoption`, which the current version lacks
```

### Retired, BundleSuperseded, BundleExpired, VersionMismatch

A bundle handle refuses a bundle that is no longer live. `Retired`: an
operator retired it with `retire bundle NAME`, which is permanent.
`BundleSuperseded`: a later open registered other bundles, and this
one is no longer among them, or `supersede bundle NAME by NEXT`
replaced it; register it again to make it live.
`BundleExpired`: its lease ran out because nothing bound or called it
for longer than the lease. `VersionMismatch`: the bundle registered
under that name pins another version than the generated code
expects; regenerate the code from the bundle file the program
registers.

```rust
use adtb::{CallOptions, CreateOptions, Log, OpenOptions, Registration};

fn main() -> Result<(), adtb::Error> {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
    ]);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![Registration::bundle(include_str!("../shelter-1.0.adb"))?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    db.query_text("retire bundle shelter-1.0", &[], &CallOptions::default())?;
    if let Err(e) = db.registered("shelter-1.0") {
        println!("{e}");
    }
    Ok(())
}
```

```text
bundle `shelter-1.0` is retired
```

### Denied

A call asserted a principal it may not: a handle opened in process
acts as the schema owner, and a call may assert an end user, but not a
service. Assert an end-user principal, or none.

## Rejected: loads and handles

### SchemaMismatch

A batch pushed to an appender does not have the appender's schema:
its fields differ in name, order or type. It stages nothing. Build the
batch from `schema()` of the appender.

```rust
use std::sync::Arc;

use adtb::batch::BatchBuilder;
use adtb::key::Value as Cell;
use adtb::{ArrowSchema, CreateOptions, LoadOptions, Log, OpenOptions, RecordBatch};

fn main() -> Result<(), adtb::Error> {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
    ]);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let mut load = db.load(&LoadOptions::default())?;
    let mut cats = load.appender("pets.Cat")?;
    let lives = cats.schema().fields[1].clone();
    let only_lives = Arc::new(ArrowSchema {
        fields: vec![lives],
    });
    let mut rows = BatchBuilder::new(only_lives.batch_schema());
    rows.push_row(&[Cell::I64(9)]);
    let batch = RecordBatch::new(only_lives, rows.finish()).expect("the batch fits");
    if let Err(e) = cats.push(&batch) {
        println!("{e}");
    }
    Ok(())
}
```

```text
schema mismatch: a batch of fields (lives) is not of the row type of shelter::pets.Cat
```

### Closed

A call, a transaction or a load reached a database after `close`,
through any handle: "the database is closed". Open the database again.

```rust
use adtb::{CallOptions, CreateOptions, Log, OpenOptions};

fn main() -> Result<(), adtb::Error> {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
    ]);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let other = db.clone();
    db.close()?;
    if let Err(e) = other.query_text("count(p for p in pets)", &[], &CallOptions::default()) {
        println!("{e}");
    }
    Ok(())
}
```

```text
the database is closed
```

## Fatal

### MissingFunction

A call reached a host function the program did not supply when it
opened the database. Supply the body in `OpenOptions::host_fns`, as
[the Rust API](rust-api.md#host-functions) shows.

```rust
use adtb::{CallOptions, CreateOptions, Log, OpenOptions, Registration};

fn main() -> Result<(), adtb::Error> {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
    ]);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![Registration::bundle(include_str!("../shelter-1.0.adb"))?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let opts = CallOptions::default();
    db.query_text("insert pets.Cat { name: \"tom\", lives: 9 }", &[], &opts)?;
    let app = db.registered("shelter-1.0")?;
    let shouted = app.ordinal("shouted").expect("a query shouted");
    if let Err(e) = app.call(shouted, &[], &opts) {
        println!("{e}");
    }
    Ok(())
}
```

```text
fatal: host function `shout` is missing
```

### Malformed

A result is not a value of the Rust type the generated code expects.
The code was generated from another schema or bundle than the one the
program registers: regenerate it.

### Storage

A read or write of the database's files failed, or the device is
full. The message carries the operating system's detail. Free space or
fix the device, then open the database again; it recovers everything
committed.

It also reports a damaged page when the engine cannot tell what holds the
page, because an index it would have to walk cannot be followed. The
message names the page, nothing is quarantined, and the next read of the
page tries again.

### Quarantined

A page failed its checksum or log position check, or `verify` found an
index wrong, and `what` names what is stopped: a constructor relation
such as `app::identities.Apple`, an index, or `the database`. The call
that finds the damage fails this way, and the mark survives a restart.

Every call reading that constructor relation fails from then on, whatever
its arguments and its plan; other calls run. A quarantined index fails no
read once marked, its queries only cost more, but a write to a constructor
it covers fails. With `the database` only `verify` runs. No command lifts a
mark yet.

## Open errors

`adtb::open` and the shell report these before any text runs. The
shell prefixes the message with the file's name.

```adt no-toms.adt
module shelter
add check no_toms on pets (name != "tom")
```

<!-- exit 1 0 1 1 -->
```console
$ adtb missing.db -c 'count(p for p in pets)'
missing.db: the file does not exist
$ adtb --create --schema migrations/0001.adt --schema migrations/0002.adt open.db -c 'insert pets.Cat { name: "tom", lives: 9 }'
$ adtb --schema migrations/0001.adt --schema migrations/0002.adt --schema no-toms.adt open.db -c 'count(p for p in pets)'
open.db: migration 3 is refused: constraint shelter::no_toms is violated by Cat { name: "tom", lives: 9 }
$ adtb --schema migrations/0002.adt open.db -c 'count(p for p in pets)'
open.db: the file's migration 1 differs from the log's
```

The check `no_toms` does not hold for the cat already in the file, so
migration 3 is refused and the file stays at migration 2. The last run
gives the second migration where the file's log has the first.

| Error | Message | What to do |
| --- | --- | --- |
| `NotFound` | the file does not exist | Check the path, or open with `create`. |
| `Locked` | the file is locked by another process, or by process N | Another process has the database open. Stop it, or reach the database through that process. |
| `AlreadyOpen` | the file is already open in this process | Share the handle you have; clones are cheap. |
| `Diverged` | the file's migration N differs from the log's | The program's log is not the file's: a migration was edited or reordered after it was applied. Restore the migration as applied, and add a new one instead. |
| `MigrationRefused` | migration N is refused: REASON | The migration does not apply to the data the file holds. Migrations before it stay applied. Fix the migration and open again. |
| `RegistrationRefused` | bundle B is refused: REASON | A bundle does not check at the version it pins, or its hash is not that version's. No bundle is registered. Fix the bundle file. |
| `HostFnMissing` | host function F is not bound | An index, constraint or key needs a host function the program did not supply. Supply it. |
| `HostFnMismatch` | host function F is bound at another type | A supplied body's version or determinism is not the declared one. Register it as declared, or ship a migration that redefines it. |
| `DowngradeRefused` | downgrade refused: CAUSE | A bundle pins an older version than the file's and would read a constructor enabled since, or a host function redefined since. Open with the newer program. |
| `Corrupt` | the file is corrupt: DETAIL | The file failed a check as it was read. Restore it from a backup. |
| `Io` | the operating system's message | Fix what it names. |

A wrong hash in a bundle header, a host function supplied at another
version than the declared one, and a second open in one process look
like this:

```rust
use adtb::{CreateOptions, Determinism, HostFns, Log, OpenOptions, Registration};

const WRONG: &str = "bundle shelter-1.1 pins 2 (0000000000000000000000000000000000000000000000000000000000000000) edition \"2026-10\"
import shelter
query names () -> Set Text = from p in pets select p.name
";

fn main() -> Result<(), adtb::Error> {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
    ]);
    let wrong_hash = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![Registration::bundle(WRONG)?],
        ..OpenOptions::default()
    };
    if let Err(e) = adtb::open("shelter.db", &log, &wrong_hash) {
        println!("{e}");
    }
    let mut host_fns = HostFns::new();
    host_fns.register("shout", "2.0", Determinism::Deterministic, |args| {
        Ok(args.clone())
    });
    let wrong_version = OpenOptions {
        create: Some(CreateOptions::default()),
        host_fns,
        ..OpenOptions::default()
    };
    if let Err(e) = adtb::open("shelter.db", &log, &wrong_version) {
        println!("{e}");
    }
    let create = OpenOptions {
        create: Some(CreateOptions::default()),
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &create).expect("shelter.db opens");
    if let Err(e) = adtb::open("shelter.db", &log, &create) {
        println!("{e}");
    }
    db.close()
}
```

```text
bundle shelter-1.1 is refused: bundle shelter-1.1 pins version 2 with hash 0000000000000000000000000000000000000000000000000000000000000000, and the log's version 2 has hash c2d244dc1b156f7f2e2cb7a03cd67d2c1aa27c156b88cde97567011c7fba4ad8
host function shout is bound at another type
the file is already open in this process
```

The second message says "type" for any mismatch, the version
included.

## Reasons this engine does not raise

The specification defines more reasons, for the server protocol,
replicas, feeds and operational commands that this engine does not run
yet. You will not meet them here: `SafePointTimeout`, `Overloaded`,
`StreamIdle`, `Fenced`, `ReplayConflict`, `Terminated`,
`TransactionExpired`, `TransactionBusy`, `MixedVersions`,
`UnknownTransaction`, `CatalogMismatch`, `TooOld`, `ReadOnlyReplica`,
`RequestExpired`, `RequestReused`, `AuthFailed`,
`UnsupportedProtocol`, `UnknownDatabase`, `EditionLocked`,
`MigrationInProgress` and `DumpDamaged`, and the open
errors `KeyRequired`, `WrongKey` and `FormatTooNew`.
