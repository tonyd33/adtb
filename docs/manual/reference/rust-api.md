# The Rust API

A Rust program can link the engine as a library and use a database
file directly, with no server between them. The program ships its
schema as an operator log and its queries and transactions as a
bundle file. Opening the database migrates the file to the log and
registers the bundle; the program then calls the bundle's queries and
transactions through Rust code generated from the bundle file.

This page builds such a program over the shelter of the
[tutorial](../tutorial/1-schema.md). Every `rust` block below is a
whole `src/main.rs`; the test suite compiles and runs each one in an
empty directory, and the output shown after a block is what it
prints. The rules behind each call are in
[the specification](../../spec/embedding.md).

## Set up a crate

The engine is the `adtb` crate. `adtb-codegen` turns a bundle file into
Rust at build time, and `adtb-lang` prints values in the textual
syntax. None of them is published yet, so depend on them by path, from
a checkout of the repository:

```toml
[dependencies]
adtb = { path = "../adtb/crates/adtb" }
adtb-lang = { path = "../adtb/crates/adtb-lang" }

[build-dependencies]
adtb-codegen = { path = "../adtb/crates/adtb-codegen" }
```

`adtb::open` opens a file on Linux only; the engine does not run on
other platforms yet.

## The migrations

In process the program, not the shell, holds the database's log: the
list of every migration, in order. Keep one file per migration in a
`migrations` directory, named so that they sort in log order. The
shell lets the first one be a schema file; here every one is a
migration file, so the first starts by adding its module.

```adt migrations/0001.adt
module shelter
add module shelter
add type Pet { name: Text } = Cat { lives: I64 } | Dog { good: Bool }
add relation pets : Pet key name
```

The second migration adds adoptions, which reference pets, and a
function whose body the program supplies, `shout`. A host function is
declared with the name and version the program registers its body
under, and whether it is deterministic.

```adt migrations/0002.adt
module shelter
add type Adoption = { pet: Key pets, by: Text }
add relation adoptions : Adoption key pet
define fn shout (t: Text) -> Text = host "shout" version "1.0" deterministic
```

## The bundle

A bundle file lists the queries and transactions the program calls.
Its header names the bundle, `shelter-1.0`, and pins the version of
the schema its text is checked at: version 2, the version the log
reaches after its second migration. Every item is checked once, when
the database opens, never when it is called.

```adt shelter-1.0.adb
bundle shelter-1.0 pins 2 (c2d244dc1b156f7f2e2cb7a03cd67d2c1aa27c156b88cde97567011c7fba4ad8) edition "2026-10"
import shelter
import fn shout
query names () -> Set Text = from p in pets select p.name
query all_pets () -> Set Pet = from p in pets select p
query pet_count () -> I64 = count(p for p in pets)
query adopted () -> Set Adoption = from a in adoptions select a
query shouted () -> Set Text = from p in pets select shout(p.name)
txn admit_cat (n: Text, lives: I64) -> Key pets =
  transact
    let c = insert pets.Cat { name: n, lives: lives }
    return key c
txn admit_dog (n: Text, good: Bool) -> Key pets =
  transact
    let d = insert pets.Dog { name: n, good: good }
    return key d
txn adopt (p: Key pets, who: Text) -> Key adoptions =
  transact
    let a = insert adoptions { pet: p, by: who }
    return key a
```

`import shelter` makes the module's types and relations visible, and
`import fn shout` makes the schema function callable. A `query` only
reads; a `txn` runs a `transact` block and commits.

The parenthesized hash after the pinned version guards against a
bundle checked at one schema and run at another. It is the hash of
that version, which `catalog::migrations` holds for each migration.
Ask the shell for it, and write the digits after `0x` into the header:

```console
$ adtb --create --schema migrations/0001.adt --schema migrations/0002.adt shelter.db -c 'from m in catalog::migrations where m.version == catalog::migrations.key(2) select m.hash'
0xc2d244dc1b156f7f2e2cb7a03cd67d2c1aa27c156b88cde97567011c7fba4ad8
```

A wrong hash refuses the open, and the error names the right one.

## Generate the code

A build script turns the log and the bundle file into Rust. Save it as
`build.rs` beside `Cargo.toml`:

```rust build.rs
use std::path::Path;

use adtb_codegen::{BundleSource, generate, read_log};

fn main() {
    let log = read_log(Path::new("migrations")).expect("the migrations read");
    let log: Vec<&str> = log.iter().map(String::as_str).collect();
    let text = std::fs::read_to_string("shelter-1.0.adb").expect("the bundle reads");
    let bundle = BundleSource {
        module: "queries",
        text: &text,
    };
    let code = generate(&log, &[bundle]).unwrap_or_else(|e| panic!("{e}"));
    let out = std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR");
    std::fs::write(Path::new(&out).join("shelter.rs"), code).expect("the code writes");
}
```

`read_log` reads every `.adt` file of the directory, in name order.
The generated file has two modules:

- `schema`: `LOG`, the text of each migration; a Rust type for each
  type of the schema at the end of the log, such as the enum `Pet`
  with variants `Cat` and `Dog`; and a unit struct for each relation,
  such as `Pets`, with one for each of its constructors, such as
  `pets::Cat`.
- `queries`, named by `BundleSource::module`: `BUNDLE`, the bundle;
  `Error`, its error type; and a unit struct for each query, transaction
  and imported function, named as the bundle file names it, such as
  `queries::admit_cat`. It re-exports `schema`'s items.

Include the file once in `src/main.rs`:

```rust fragment
mod shelter {
    include!(concat!(env!("OUT_DIR"), "/shelter.rs"));
}
```

## Open and call

`adtb::open` takes the file's path, the log and the open options. With
`create` set it creates the file when it does not exist; without it, a
missing file fails the open. `registrations` lists the bundles to
register. Opening applies the migrations of the log the file has not
seen, checks and registers the bundles, and returns the database
handle.

```rust
use adtb::{BundleDef, CallOptions, CreateOptions, Log, OpenOptions};

mod shelter {
    include!(concat!(env!("OUT_DIR"), "/shelter.rs"));
}

use shelter::{queries, schema};

fn main() -> Result<(), queries::Error> {
    let log = Log::new(schema::LOG.iter().copied());
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![queries::BUNDLE.registration()?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let app = db.bundle(&queries::BUNDLE)?;
    let opts = CallOptions::default();
    app.call(queries::admit_cat, ("tom".into(), 9), &opts)?;
    app.call(queries::admit_dog, ("rex".into(), true), &opts)?;
    let mut names = app
        .call(queries::names, (), &opts)?
        .collect::<Result<Vec<String>, _>>()?;
    names.sort();
    println!("{names:?}");
    db.close()?;
    Ok(())
}
```

```text
["rex", "tom"]
```

`db.bundle` binds the bundle: every call through the handle it returns
runs at the version the bundle pins. `call` takes the generated value
of a query or transaction, its arguments as a tuple, and the call
options. A query's result is an iterator of rows; a set's elements come
in no promised order, so sort them when order matters. `close` waits
for running calls, takes a checkpoint and releases the file.

The handle is cheap to clone, and clones share one engine; use it from
as many threads as you like. Each call runs as a serializable
transaction of its own.

## Values

Each language type has a Rust type:

| Language | Rust |
| --- | --- |
| `I8` to `I64` | `i8` to `i64` |
| `Text` | `String` |
| `Bool` | `bool` |
| `Option a` | `Option<A>` |
| `Set a` | `Vec<A>`, its elements distinct and in ascending order (a query's own rows come in no promised order) |
| `Key r` | `adtb::Key<R>`, holding the key's value |
| a declared type | a generated enum or struct |
| `Uuid`, `Bytes`, `Decimal`, the time types | the types of `adtb::typed` |

A declared sum is an enum with one variant per constructor, each
holding the header fields and then its own; a record type is a struct.
A query's result is `Rows<T, queries::Error>`, an iterator of
`Result<T, queries::Error>`; a query whose result is not a set, such as
a count, returns its value. A transaction returns
`Committed<T>`: its result `value`, the commit `position`, and the
`attempts` it took.

```rust
use adtb::{BundleDef, CallOptions, CreateOptions, Log, OpenOptions};

mod shelter {
    include!(concat!(env!("OUT_DIR"), "/shelter.rs"));
}

use shelter::schema::Pet;
use shelter::{queries, schema};

fn main() -> Result<(), queries::Error> {
    let log = Log::new(schema::LOG.iter().copied());
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![queries::BUNDLE.registration()?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let app = db.bundle(&queries::BUNDLE)?;
    let opts = CallOptions::default();
    let tom = app.call(queries::admit_cat, ("tom".into(), 9), &opts)?;
    println!("{:?} after {} attempt", tom.value, tom.attempts);
    app.call(queries::admit_dog, ("rex".into(), true), &opts)?;
    app.call(queries::adopt, (tom.value, "ann".into()), &opts)?;
    let mut pets = app
        .call(queries::all_pets, (), &opts)?
        .collect::<Result<Vec<Pet>, _>>()?;
    pets.sort_by_key(|p| match p {
        Pet::Cat { name, .. } | Pet::Dog { name, .. } => name.clone(),
    });
    for pet in pets {
        match pet {
            Pet::Cat { name, lives } => println!("{name} is a cat with {lives} lives"),
            Pet::Dog { name, good } => println!("{name} is a dog, good: {good}"),
        }
    }
    for a in app.call(queries::adopted, (), &opts)? {
        let a = a?;
        println!("{} adopted {}", a.by, a.pet.0);
    }
    println!("{} pets", app.call(queries::pet_count, (), &opts)?);
    Ok(())
}
```

```text
Key<shelter::pets>("tom") after 1 attempt
rex is a dog, good: true
tom is a cat with 9 lives
ann adopted tom
2 pets
```

`Key<Pets>` holds the key's value, here the pet's name, in its field
`0`. Pass it back to a call that takes a `Key pets`, as `adopt` does.

## Calls by ordinal

`Database::registered` returns the handle of a registered bundle by
name, without generated code. Its calls take a callable by ordinal and
its arguments as `adtb::Value`s, and return untyped rows. `ordinal`
looks an ordinal up by name. A query returns `Called::Rows`, a
transaction `Called::Committed`; the rows carry their type, and
`adtb_lang::textual::print` prints a row in the textual syntax:

```rust
use adtb::{BundleDef, CallOptions, Called, CreateOptions, Log, OpenOptions, Value};
use adtb_lang::textual::print;

mod shelter {
    include!(concat!(env!("OUT_DIR"), "/shelter.rs"));
}

use shelter::{queries, schema};

fn main() -> Result<(), adtb::Error> {
    let log = Log::new(schema::LOG.iter().copied());
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![queries::BUNDLE.registration()?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let app = db.registered("shelter-1.0")?;
    let opts = CallOptions::default();
    let admit = app.ordinal("admit_cat").expect("a transaction admit_cat");
    let args = [Value::text("tom"), Value::Int(9)];
    if let Called::Committed(done) = app.call(admit, &args, &opts)? {
        println!("committed after {} attempt", done.attempts);
    }
    let all = app.ordinal("all_pets").expect("a query all_pets");
    if let Called::Rows(rows) = app.call(all, &[], &opts)? {
        let form = rows.form().clone();
        for row in rows {
            println!("{}", print(&row?, &form).expect("a row prints"));
        }
    }
    Ok(())
}
```

```text
committed after 1 attempt
Cat { name: "tom", lives: 9 }
```

Ordinals count from 1: each `import fn` in order, then each query and
transaction in order. Here `shout` is 1 and `names` is 2. Generated code
computes the same ordinals, so the two kinds of call agree.

## Ad hoc text

`query_text` runs ad hoc text, as the shell does, in one transaction.
It returns one set of rows for each item that yields a result. A free
name in the text is a parameter; give each one as a name, its type and
a value:

```rust
use adtb::typed::Base;
use adtb::{CallOptions, CreateOptions, Log, OpenOptions, Value};
use adtb_lang::textual::print;
use adtb_lang::types::Ty;

mod shelter {
    include!(concat!(env!("OUT_DIR"), "/shelter.rs"));
}

use shelter::schema;

fn main() -> Result<(), adtb::Error> {
    let log = Log::new(schema::LOG.iter().copied());
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let opts = CallOptions::default();
    db.query_text(
        "insert pets.Cat { name: \"tom\", lives: 9 }
         insert pets.Cat { name: \"kit\", lives: 3 }",
        &[],
        &opts,
    )?;
    let params = [("least", Ty::Base(Base::I64), Value::Int(5))];
    let results = db.query_text(
        "from c in pets.Cat where c.lives >= least select c.name order by c.name",
        &params,
        &opts,
    )?;
    for rows in results {
        let form = rows.form().clone();
        for row in rows {
            println!("{}", print(&row?, &form).expect("a row prints"));
        }
    }
    Ok(())
}
```

```text
"tom"
```

The first text holds two inserts; they commit together or not at all.
A program that opens a database with no bundle can still read and
write it this way.

## Transactions

`begin` on a bundle handle starts an interactive transaction. Its calls
run inside it, one at a time, and see each other's writes; nothing is
visible to other transactions until `commit`, which returns the commit
position. Inside a transaction a registered transaction returns
`Pending<T>`, which has no position of its own. Dropping the
transaction, or calling `rollback`, discards it.

`transact` runs a closure in a new transaction and commits it, running
it again in a fresh transaction when it fails with a serialization
failure, at most the given number of attempts in all. The closure may
run more than once, so it must have no effects outside the database.

```rust
use adtb::{BundleDef, CallOptions, CreateOptions, Log, OpenOptions, TxnCallOptions, TxnOptions};

mod shelter {
    include!(concat!(env!("OUT_DIR"), "/shelter.rs"));
}

use shelter::{queries, schema};

fn main() -> Result<(), queries::Error> {
    let log = Log::new(schema::LOG.iter().copied());
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![queries::BUNDLE.registration()?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let app = db.bundle(&queries::BUNDLE)?;
    let opts = TxnCallOptions::default();

    let mut t = app.begin(&TxnOptions::default())?;
    let tom = t.call(queries::admit_cat, ("tom".into(), 9), &opts)?;
    t.call(queries::adopt, (tom.value, "ann".into()), &opts)?;
    let count = t.call(queries::pet_count, (), &opts)?;
    println!("{count} pet inside the transaction");
    t.rollback()?;
    let count = app.call(queries::pet_count, (), &CallOptions::default())?;
    println!("{count} pets after the rollback");

    let done = app.transact(&TxnOptions::default(), 3, |t| {
        let rex = t.call(queries::admit_dog, ("rex".into(), true), &opts)?;
        t.call(queries::adopt, (rex.value.clone(), "bo".into()), &opts)?;
        Ok(rex.value)
    })?;
    println!("{:?} committed after {} attempt", done.value, done.attempts);
    Ok(())
}
```

```text
1 pet inside the transaction
0 pets after the rollback
Key<shelter::pets>("rex") committed after 1 attempt
```

`Database::begin` starts an ad hoc transaction instead, whose
`query_text` runs ad hoc text inside it; it commits and rolls back the
same way, and `Database::transact` reruns a closure over one. The engine never reruns an interactive transaction on its
own: a serialization failure ends it, and the error says to retry.
[Serializable transactions](../concepts/serializable-transactions.md)
explains when that happens.

## Cancellation and deadlines

Every call takes options. `cancel` holds a `CancelToken`: cancelling
it, from any thread, stops every call that carries it, and a call
started with a cancelled token does not run. `deadline` is a wall-clock
time after which the call stops, and `limit` a time limit for each of
its statements. A call stopped any of these ways has no effect.

```rust
use std::time::SystemTime;

use adtb::{BundleDef, CallOptions, CancelToken, CreateOptions, Log, OpenOptions};

mod shelter {
    include!(concat!(env!("OUT_DIR"), "/shelter.rs"));
}

use shelter::{queries, schema};

fn main() -> Result<(), queries::Error> {
    let log = Log::new(schema::LOG.iter().copied());
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![queries::BUNDLE.registration()?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let app = db.bundle(&queries::BUNDLE)?;

    let token = CancelToken::new();
    let cancelled = CallOptions {
        cancel: Some(token.clone()),
        ..CallOptions::default()
    };
    token.cancel();
    if let Err(queries::Error::Rejected { reason }) =
        app.call(queries::admit_cat, ("tom".into(), 9), &cancelled)
    {
        println!("{reason}");
    }

    let passed = CallOptions {
        deadline: Some(SystemTime::UNIX_EPOCH),
        ..CallOptions::default()
    };
    if let Err(queries::Error::Rejected { reason }) =
        app.call(queries::admit_cat, ("tom".into(), 9), &passed)
    {
        println!("{reason}");
    }
    let count = app.call(queries::pet_count, (), &CallOptions::default())?;
    println!("{count} pets");
    Ok(())
}
```

```text
the call was cancelled
the call's deadline passed
0 pets
```

The second call's deadline, the Unix epoch, passed long ago. A
running query notices a cancellation or a deadline between batches
of rows. A call that has committed stays committed whatever the token
does after. Inside an interactive transaction, a stopped call leaves
the transaction open without its effects. `TxnOptions` takes a deadline
and a limit for every call of a transaction, and `TxnCallOptions` for
one call in it.

## Arrow results

`call_arrow` runs a call as `call` does and returns its result as a
stream of record batches in Arrow's columnar layout, all of one schema.
A record result has one column per field, in declared order; a sum of
two or more constructors has one column, `row`; any other result has
one column, `value`. The batches are the engine's own type,
`adtb::RecordBatch`, laid out as Arrow lays out columns; the engine
does not depend on the `arrow` crate.

```rust
use adtb::{BundleDef, CallOptions, CreateOptions, Log, OpenOptions};

mod shelter {
    include!(concat!(env!("OUT_DIR"), "/shelter.rs"));
}

use shelter::{queries, schema};

fn main() -> Result<(), queries::Error> {
    let log = Log::new(schema::LOG.iter().copied());
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![queries::BUNDLE.registration()?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let app = db.bundle(&queries::BUNDLE)?;
    let opts = CallOptions::default();
    let tom = app.call(queries::admit_cat, ("tom".into(), 9), &opts)?;
    app.call(queries::adopt, (tom.value, "ann".into()), &opts)?;

    let stream = app.call_arrow(queries::adopted, (), &opts)?;
    let names: Vec<String> = stream
        .schema()
        .fields
        .iter()
        .map(|f| f.name.clone())
        .collect();
    println!("columns: {names:?}");
    for batch in stream {
        let batch = batch?;
        for row in 0..batch.num_rows() {
            println!("{:?}", batch.batch().row(row));
        }
    }
    Ok(())
}
```

```text
columns: ["pet", "by"]
[Text("tom"), Text("ann")]
```

`batch()` returns the columns themselves, an `adtb::batch::Batch`;
`row` and `value` read them a row or a cell at a time.

## Bulk append

A load streams batches into constructor relations and inserts them as
one change set when it commits. Open a load, take an appender for each
constructor relation you fill, push batches, and commit. The appender's
`schema()` is the schema its batches must have: one field per field of
the row, in declared order. Fields a key generator or a default fills
may be left out.

```rust
use std::sync::Arc;

use adtb::batch::BatchBuilder;
use adtb::key::Value as Cell;
use adtb::{BundleDef, CallOptions, CreateOptions, LoadOptions, Log, OpenOptions, RecordBatch};

mod shelter {
    include!(concat!(env!("OUT_DIR"), "/shelter.rs"));
}

use shelter::{queries, schema};

fn main() -> Result<(), queries::Error> {
    let log = Log::new(schema::LOG.iter().copied());
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![queries::BUNDLE.registration()?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");

    let mut load = db.load(&LoadOptions::default())?;
    let mut cats = load.appender(schema::pets::Cat)?;
    let fields = cats.schema().clone();
    let names: Vec<&str> = fields.fields.iter().map(|f| f.name.as_str()).collect();
    println!("fields: {names:?}");
    let mut rows = BatchBuilder::new(fields.batch_schema());
    for (name, lives) in [("tom", 9), ("kit", 3), ("tab", 7)] {
        rows.push_row(&[Cell::Text(name.into()), Cell::I64(lives)]);
    }
    let batch = RecordBatch::new(Arc::clone(&fields), rows.finish()).expect("the batch fits");
    cats.push(&batch)?;
    let report = load.commit()?;
    println!("{} rows", report.rows);

    let app = db.bundle(&queries::BUNDLE)?;
    let count = app.call(queries::pet_count, (), &CallOptions::default())?;
    println!("{count} pets");
    Ok(())
}
```

```text
fields: ["name", "lives"]
3 rows
3 pets
```

Pushed rows are staged unchecked. Commit checks them as one change set,
keys, references and constraints included, and inserts all of them or
none. A load is one write transaction; the engine does not rerun it.

## Host functions

A function declared with a `host` body runs code the program supplies
when it opens the database. Register the body in `HostFns` under the
declared name and version, with the declared determinism, and pass it
in the open options. The body takes a batch with one column per
parameter and returns a batch of one column of the result type, with
as many rows; it may be called from several threads at once.

```rust
use adtb::batch::{Batch, BatchBuilder};
use adtb::key::Value as Cell;
use adtb::{
    BundleDef, CallOptions, CreateOptions, Determinism, HostError, HostFns, Log, OpenOptions,
};

mod shelter {
    include!(concat!(env!("OUT_DIR"), "/shelter.rs"));
}

use shelter::{queries, schema};

fn shout(args: &Batch) -> Result<Batch, HostError> {
    let mut out = BatchBuilder::new(args.schema().clone());
    for row in 0..args.len() {
        let Cell::Text(t) = args.value(0, row) else {
            return Err(HostError("shout takes text".into()));
        };
        out.push_row(&[Cell::Text(t.to_uppercase())]);
    }
    Ok(out.finish())
}

fn main() -> Result<(), queries::Error> {
    let log = Log::new(schema::LOG.iter().copied());
    let mut host_fns = HostFns::new();
    host_fns.register("shout", "1.0", Determinism::Deterministic, shout);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![queries::BUNDLE.registration()?],
        host_fns,
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let app = db.bundle(&queries::BUNDLE)?;
    let opts = CallOptions::default();
    app.call(queries::admit_cat, ("tom".into(), 9), &opts)?;
    for name in app.call(queries::shouted, (), &opts)? {
        println!("{}", name?);
    }
    Ok(())
}
```

```text
TOM
```

A body that returns an error or panics faults the statement that
called it. Opening without a body for a declared host function
succeeds unless an index, constraint or key reaches it; a call that
reaches it then fails. The version is part of the function's identity:
once a database has bound `shout` at `1.0`, opening it with a body at
another version fails, unless a migration redefines the function at
that version.

## Bundles across releases

A bundle pins the schema version it was checked at, and keeps working
as later migrations change the schema. The engine translates each call
from the pinned version to the current one: its arguments forward, its
results back. Here the log gains a third migration that renames
`Pet.name` to `called`, and the bundle, still pinned at version 2,
runs unchanged. Ad hoc text runs at the current version and sees the
new name:

```rust
use adtb::{CallOptions, Called, CreateOptions, Log, OpenOptions, Registration, Value};
use adtb_lang::textual::print;

const RENAME: &str = "module shelter
rename field Pet.name to called
";

fn main() -> Result<(), adtb::Error> {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
        RENAME,
    ]);
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![Registration::bundle(include_str!("../shelter-1.0.adb"))?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &options).expect("shelter.db opens");
    let app = db.registered("shelter-1.0")?;
    println!("pinned at {:?}", app.pins());
    let opts = CallOptions::default();
    let admit = app.ordinal("admit_cat").expect("a transaction admit_cat");
    app.call(admit, &[Value::text("tom"), Value::Int(9)], &opts)?;
    let names = app.ordinal("names").expect("a query names");
    if let Called::Rows(rows) = app.call(names, &[], &opts)? {
        let form = rows.form().clone();
        for row in rows {
            println!("{}", print(&row?, &form).expect("a row prints"));
        }
    }
    for rows in db.query_text("from p in pets select p", &[], &opts)? {
        let form = rows.form().clone();
        for row in rows {
            println!("{}", print(&row?, &form).expect("a row prints"));
        }
    }
    Ok(())
}
```

```text
pinned at Some(2)
"tom"
Cat { called: "tom", lives: 9 }
```

Renames, added fields and constructors, drops of fields the bundle
does not read, and the operators that leave data alone all translate.
When a query of the bundle reaches what a split, merge, nest, unnest,
move field, alter field or rekey changes, reads a field a later
migration drops, takes a parameter whose type gains or loses a field,
reads the past or reads files, it has no translation: the open that registers the bundle fails with
`Untranslatable`, naming the query and the operator
([Errors](errors.md#untranslatable)).

### Supersede and retire

Each open registers the bundles it is given and supersedes every other
bundle an earlier open registered. A superseded bundle is not live: it
holds back no schema change, and its handles refuse calls with
`BundleSuperseded`. Giving it to a later open makes it live again. The
shell opens a file with no bundles, so opening a program's file in the
shell supersedes the program's bundles until the program opens it
again.

Two commands of ad hoc text change a bundle's state while the database
is open. `supersede bundle B by B2` supersedes `B` in favour of `B2`, a
live bundle of the same family: the part of the name before the
version. `retire bundle B` retires `B` for good; a retired bundle's
name never registers again. Each returns the bundle's row of
`catalog::bundles`.

```rust
use adtb::{CallOptions, CreateOptions, Log, OpenOptions, Registration};

const NEXT: &str = "bundle shelter-1.1 pins 2 (c2d244dc1b156f7f2e2cb7a03cd67d2c1aa27c156b88cde97567011c7fba4ad8) edition \"2026-10\"
import shelter
query names () -> Set Text = from p in pets select p.name
";

fn main() -> Result<(), adtb::Error> {
    let log = Log::new([
        include_str!("../migrations/0001.adt"),
        include_str!("../migrations/0002.adt"),
    ]);
    let both = OpenOptions {
        create: Some(CreateOptions::default()),
        registrations: vec![
            Registration::bundle(include_str!("../shelter-1.0.adb"))?,
            Registration::bundle(NEXT)?,
        ],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &both).expect("shelter.db opens");
    let opts = CallOptions::default();
    db.query_text("supersede bundle shelter-1.0 by shelter-1.1", &[], &opts)?;
    if let Err(e) = db.registered("shelter-1.0") {
        println!("{e}");
    }
    db.close()?;
    let old = OpenOptions {
        registrations: vec![Registration::bundle(include_str!("../shelter-1.0.adb"))?],
        ..OpenOptions::default()
    };
    let db = adtb::open("shelter.db", &log, &old).expect("shelter.db opens again");
    db.registered("shelter-1.0")?;
    println!("shelter-1.0 is live again");
    if let Err(e) = db.registered("shelter-1.1") {
        println!("{e}");
    }
    Ok(())
}
```

```text
bundle `shelter-1.0` is superseded
shelter-1.0 is live again
bundle `shelter-1.1` is superseded
```

## Open options

| Field | Meaning |
| --- | --- |
| `create` | Create the file when it does not exist. |
| `registrations` | The bundles to register; all of them register, or the open fails. |
| `host_fns` | The bodies of the schema's host functions. |
| `pool_bytes` | The size of the page cache. |
| `query_memory_bytes` | The memory queries may use before they spill to disk. |
| `workers` | The worker threads. |
| `fix_rounds` | The rounds a fixpoint may take before its statement is stopped. |
| `file_root` | The directory a file scan resolves a relative path against; the working directory by default. |

The settings hold until the database closes. The query
`from o in sys::process_options select o` shows each one.

## Closing

`close` waits for running calls to finish, rolls back interactive
transactions still open, writes a checkpoint and releases the file.
Closing any handle closes the database for every clone and every
bundle handle: later calls fail with "the database is closed".
Dropping the last handle closes the database too, and a process that
exits without closing loses nothing it committed; the next open
recovers.

One process at a time may open a file. A second open from another
process fails with "the file is locked by another process", or names
the process when the system reports it, and a second
open from the same process with "the file is already open in this
process": share the handle instead.

The errors every call can return, and what to do about each, are in
[Errors](errors.md).
