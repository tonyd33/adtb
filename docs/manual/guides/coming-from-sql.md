# Coming from SQL

This guide is for a reader who knows SQLite or another SQL database. It
maps what you know onto adtb, works one schema through both, and moves
its data out of SQLite with files you can run today. The SQL here was
run by hand on `sqlite3` 3.54.0, because the manual's checks run only
the `adtb` shell. Statements about SQLite link to its documentation.

## What you know, what to use

| In SQL | In adtb |
| --- | --- |
| Table | A [`relation`](../tutorial/1-schema.md) of a declared type. Each row is one constructor of that type |
| `PRIMARY KEY` | `key field` or `key (a, b)`. Every relation has exactly one, and a row's key never changes |
| `FOREIGN KEY` | A field of type [`Key r`](../tutorial/5-references-and-constraints.md), always enforced. [SQLite leaves foreign keys off](https://www.sqlite.org/foreignkeys.html) until each connection turns them on |
| `ON DELETE CASCADE`, `SET NULL` | `on delete cascade`, and `on delete clear` on an `Option (Key r)` field. With nothing written, the delete is refused |
| Nullable column | An `Option` field, or a field only [some constructors](modelling-with-sums.md#optional-facts) declare |
| `NOT NULL` | Every field that is not an `Option` |
| A type column | The constructor itself; `tag(x)` reads it |
| `CHECK` | `check name on r (condition)` for one row, `constraint name = query` for anything larger ([Writing constraints](writing-constraints.md)) |
| `UNIQUE` | `unique (a, b)`, or `unique (a) in Ctor` for one constructor. `None` is one value, so a unique admits one `None`. [SQLite treats NULLs as distinct](https://www.sqlite.org/lang_createtable.html); to get that, write a [constraint that skips `None`](../tutorial/5-references-and-constraints.md#checks-and-constraints) |
| `CREATE INDEX` | `index (a)` on the relation, or `add index` in a migration. Reference fields are indexed for you |
| View | A `fn` with no parameters that returns a set ([Tutorial 3](../tutorial/3-queries.md)) |
| Trigger | Nothing runs on change. State that follows a write is written in the same [`transact` block](../tutorial/6-transactions.md); delete actions are the only writes the engine derives |
| `AUTOINCREMENT` | `key id generate sequence` on a field of type `Key r`. Values are unique and never reused, and may have gaps |
| `BEGIN`, `COMMIT` | `\begin` and `\commit` in the shell, or a `transact` block. Each statement is atomic by itself |
| `EXPLAIN` | `explain Q` prints the plan, and `explain analyze Q` also runs it and discards its effects |
| `ALTER TABLE` | A [migration](../tutorial/7-migrations.md): `add field`, a rename, `add` of a constructor, and more, recorded in a log |
| `SELECT` | [`from x in r where c select e`](../tutorial/3-queries.md) |
| Outer join | A [nested set](../concepts/no-null.md#the-missing-side-of-a-join) of partners, which may be empty |

## What has no equivalent

- NULL. `null` is not a word of the language, an insert gives every
  field or uses a declared default, and `==` is always true or false.
  [No NULL](../concepts/no-null.md) goes through each use of it.
- The SQL language. The engine parses no SQL, so a tool that speaks SQL
  cannot connect to it.
- Collection fields. A field holds one value, and a many-valued fact is
  [rows of another relation](modelling-with-sums.md#many-valued-facts).
  A query can still return them nested.
- Wildcards in `match` over a sum. A `match` names every constructor,
  so adding a constructor stops each query that reads the sum from
  checking until it handles the new case. Wildcards work only on scalars.
- Weaker isolation levels. [Every transaction is
  serializable](../concepts/serializable-transactions.md), and there is
  no setting to lower it. A transaction may abort with a serialization
  failure, which is safe to retry from the start.
- Triggers, stored procedures and a hidden row id.

## One schema, both ways

A shop records payments. A payment by card has the card's last four
digits, a transfer has an IBAN, and cash has neither. The usual SQL
design is one table with a discriminator and nullable columns. Save
this as `shop.sql`:

```sql shop.sql
CREATE TABLE customers (
  id    INTEGER PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  phone TEXT
);

CREATE TABLE payments (
  id         INTEGER PRIMARY KEY,
  customer   INTEGER NOT NULL REFERENCES customers (id),
  amount     INTEGER NOT NULL CHECK (amount > 0),
  kind       TEXT NOT NULL CHECK (kind IN ('card', 'transfer', 'cash')),
  card_last4 TEXT,
  iban       TEXT
);

INSERT INTO customers VALUES
  (1, 'ann@example.com', '555-0100'),
  (2, 'bo@example.com', NULL);

INSERT INTO payments VALUES
  (1, 1, 1200, 'card', '4242', NULL),
  (2, 1, 800, 'transfer', NULL, 'DE89370400440532013000'),
  (3, 2, 300, 'cash', NULL, NULL);
```

```sh
sqlite3 shop.sqlite < shop.sql
```

In adtb, the columns every payment has go in the header, and each kind
is a constructor with the fields it has. Save this as `shop.adt`:

```adt shop.adt
module shop

type Customer = { id: Key customers, email: Text, phone: Option Text }
relation customers : Customer key id generate sequence
  unique (email)

type Payment { id: Key payments, customer: Key customers, amount: I64 }
  = Card { last4: Text }
  | Transfer { iban: Text }
  | Cash
relation payments : Payment key id generate sequence

check positive_amount on payments (amount > 0)
```

```console
$ adtb --create --schema shop.adt shop.db
adtb> insert customers { email: "ann@example.com", phone: Some { value: "555-0100" } }
adtb> insert customers { email: "bo@example.com", phone: None }
adtb> insert payments.Card { customer: customers.key(1), amount: 1200, last4: "4242" }
adtb> insert payments.Transfer { customer: customers.key(1), amount: 800, iban: "DE89370400440532013000" }
adtb> insert payments.Cash { customer: customers.key(2), amount: 300 }
adtb> \q
```

### A query

How was each payment made? In SQL, the query repeats which column
belongs to which kind:

```sql
SELECT id, CASE kind
             WHEN 'card' THEN card_last4
             WHEN 'transfer' THEN iban
             ELSE 'cash'
           END AS how
FROM payments
ORDER BY id;
```

In adtb, the `match` names each constructor, and each arm sees only
that constructor's fields:

```console
$ adtb shop.db
adtb> from p in payments select { id: key p, how: match p | Card c -> c.last4 | Transfer t -> t.iban | Cash _ -> "cash" } order by key p
{ how: "4242", id: payments.key(1) }
{ how: "DE89370400440532013000", id: payments.key(2) }
{ how: "cash", id: payments.key(3) }
adtb> \q
```

### A write

Payment 2 is paid again by card. In SQL, the update has to clear the
old column as well as set the new one, and nothing notices if it does
not:

```sql
UPDATE payments
SET kind = 'card', card_last4 = '4242', iban = NULL
WHERE id = 2;
```

In adtb, this is a retag. The new value is a value of the other
constructor, so `iban` goes with `Transfer`, and the row keeps its key:

```console
$ adtb shop.db
adtb> update p in payments.Transfer where key p == payments.key(2) set Card { p with last4: "4242" }
adtb> from p in payments select p order by key p
Card { id: payments.key(1), customer: customers.key(1), amount: 1200, last4: "4242" }
Card { id: payments.key(2), customer: customers.key(1), amount: 800, last4: "4242" }
Cash { id: payments.key(3), customer: customers.key(2), amount: 300 }
adtb> \q
```

### The row SQL allows

As declared, the table accepts a card payment with no card number and
an IBAN. It also accepts a payment by a customer who does not exist,
because foreign keys are off unless the connection turns them on:

```sh
sqlite3 shop.sqlite "INSERT INTO payments VALUES (4, 2, 500, 'card', NULL, 'DE89370400440532013000');"
sqlite3 shop.sqlite "INSERT INTO payments VALUES (5, 9, 100, 'cash', NULL, NULL);"
```

Both succeed. A table `CHECK` can refuse the first if you write one for
each column and kind; `CHECK ((kind = 'card') = (card_last4 IS NOT NULL))`
does it for `card_last4`.

In adtb, these rows have no spelling. Each attempt below is refused
before anything changes:

<!-- exit 1 -->
```console
$ adtb shop.db
adtb> insert payments.Card { customer: customers.key(2), amount: 500, iban: "DE89370400440532013000" }
`Card` has no field `iban`
adtb> insert payments.Card { customer: customers.key(2), amount: 500 }
field `last4` of `Card` is missing, and `payments` has no default for it
adtb> insert payments.Cash { customer: customers.key(9), amount: 100 }
constraint `shop::payments.customer` is violated by 1 row
adtb> from p in payments select p.last4
Payment has no field `last4`; only header fields are reached on the whole sum: match on the constructor
adtb> \q
```

The last line is the same rule for a read: `last4` belongs to `Card`,
so a query over every payment has to `match`.

## Move your data today

There is no tool that reads a SQL schema or a SQLite file. You write
the adtb schema yourself, export each table to CSV, and load each file
with an `insert`. These steps continue from `shop.sqlite` above, which
now holds the two bad rows.

### 1. Export each table

```sh
sqlite3 -csv -header shop.sqlite "SELECT * FROM customers" > customers.csv
sqlite3 -csv -header shop.sqlite "SELECT * FROM payments" > payments.csv
```

`sqlite3` ends these lines with CRLF, which adtb reads. The files hold:

```csv customers.csv
id,email,phone
1,ann@example.com,555-0100
2,bo@example.com,
```

```csv payments.csv
id,customer,amount,kind,card_last4,iban
1,1,1200,card,4242,
2,1,800,transfer,,DE89370400440532013000
3,2,300,cash,,
4,2,500,card,,DE89370400440532013000
5,9,100,cash,,
```

A NULL exports as an empty cell and an empty string as `""`. adtb reads
the empty cell as `None` and `""` as `Some { value: "" }` in an `Option`
field, so the two stay different.

### 2. Declare the schema and the shape of the file

A file has no sums, so `PaymentRow` is the shape of a line of the file:
a type with no relation. Its `id` and `customer` read from the integer
columns, and a key read from a file keeps its value. Save this as
`moved.adt`:

```adt moved.adt
module shop

type Customer = { id: Key customers, email: Text, phone: Option Text }
relation customers : Customer key id generate sequence
  unique (email)

type Payment { id: Key payments, customer: Key customers, amount: I64 }
  = Card { last4: Text }
  | Transfer { iban: Text }
  | Cash
relation payments : Payment key id generate sequence

check positive_amount on payments (amount > 0)

type PaymentRow = { id: Key payments, customer: Key customers, amount: I64, kind: Text, card_last4: Option Text, iban: Option Text }

relation load_rejects : RejectedRow key (load, line)
```

### 3. Load the tables that are referred to first

A reference must have its target when the statement ends, so load
`customers` before `payments`. `Customer` matches the file's columns,
so the file reads at that type directly:

```console
$ adtb --create --schema moved.adt moved.db
adtb> insert customers from (from c in read_csv("customers.csv", Customer) select c)
adtb> from c in customers select c order by key c
Customer { id: customers.key(1), email: "ann@example.com", phone: Some { value: "555-0100" } }
Customer { id: customers.key(2), email: "bo@example.com", phone: None }
adtb> \q
```

### 4. Route rows onto constructors

Now match on `kind`, and on the column each kind needs. `reject` sends
a row that does not fit to `load_rejects`, with your reason. Row 5
refers to a customer that does not exist, and a broken constraint is
not a reject: it fails the whole statement and nothing lands.

<!-- exit 1 -->
```console
$ adtb moved.db
adtb> insert payments from (from r in read_csv("payments.csv", PaymentRow) select match r.kind | "card" -> (match r.card_last4 | Some { value: d } -> Card { id: r.id, customer: r.customer, amount: r.amount, last4: d } | None -> reject "card with no card_last4") | "transfer" -> (match r.iban | Some { value: i } -> Transfer { id: r.id, customer: r.customer, amount: r.amount, iban: i } | None -> reject "transfer with no iban") | "cash" -> Cash { id: r.id, customer: r.customer, amount: r.amount } | _ -> reject "unknown kind") rejects into load_rejects
constraint `shop::payments.customer` is violated by 1 row
adtb> \q
```

Find these rows in SQLite before you export, with `PRAGMA
foreign_key_check`:

```sh
sqlite3 shop.sqlite "PRAGMA foreign_key_check"
```

```text
payments|5|customers|0
```

Fix them there, or keep them out of the load with a `where` in the
source query, as this does:

```console
$ adtb moved.db
adtb> insert payments from (from r in read_csv("payments.csv", PaymentRow) where exists c in customers where key c == r.customer select match r.kind | "card" -> (match r.card_last4 | Some { value: d } -> Card { id: r.id, customer: r.customer, amount: r.amount, last4: d } | None -> reject "card with no card_last4") | "transfer" -> (match r.iban | Some { value: i } -> Transfer { id: r.id, customer: r.customer, amount: r.amount, iban: i } | None -> reject "transfer with no iban") | "cash" -> Cash { id: r.id, customer: r.customer, amount: r.amount } | _ -> reject "unknown kind") rejects into load_rejects
adtb> from j in load_rejects select { line: j.line, raw: j.raw, reason: j.reason } order by j.line
{ line: 4, raw: "4,2,500,card,,DE89370400440532013000", reason: "card with no card_last4" }
adtb> from p in payments select p order by key p
Card { id: payments.key(1), customer: customers.key(1), amount: 1200, last4: "4242" }
Transfer { id: payments.key(2), customer: customers.key(1), amount: 800, iban: "DE89370400440532013000" }
Cash { id: payments.key(3), customer: customers.key(2), amount: 300 }
adtb> \q
```

The card with no number is in `load_rejects`, and the rest of the file
landed. A `where` drops a row without a record, so count what it drops
before you rely on it. [Files and CSV scans](files-and-csv.md) has more
on loads, including staging rows for repair.

### 5. Check the generated keys carry on

Loaded rows keep their ids, and the sequence continues above them:

```console
$ adtb moved.db -c 'insert customers { email: "cy@example.com", phone: None }'
$ adtb moved.db -c 'from c in customers select c.id order by c.id'
customers.key(1)
customers.key(2)
customers.key(3)
```

### What you map by hand

- The schema. Decide for each nullable column whether it is an
  `Option` or belongs to a constructor, and for each type column which
  sum it becomes.
- A NULL in a column you declared `Text`. An empty cell reads as the
  empty string with no error, so a NULL you did not expect passes
  silently. A number, date or `Bool` field fails on an empty cell. Check
  first with `SELECT count(*) FROM t WHERE c IS NULL`.
- Values with another textual form. Change them in the export's
  `SELECT`, as the table below shows. A raw `BLOB` that is not UTF-8
  fails the whole load, even with a rejects relation. A value of the
  wrong type, which [SQLite's dynamic
  typing](https://www.sqlite.org/datatype3.html) allows, fails
  conversion and goes to the rejects relation.
- A reference to a composite key. It reads from one column per key
  field, named by path: a field `t: Key ts` with key `(a, b)` reads
  from `t.a` and `t.b`, so alias the export's columns to those names.
- A `UNIQUE` column with several NULLs. Declared `unique`, it refuses
  the second `None`; write the constraint from the table above.
- Indexes, views and triggers. Nothing carries them over. Declare the
  indexes and functions in the schema, and move trigger logic into the
  transactions that write.

| Column in SQLite | Field in adtb | Export it as |
| --- | --- | --- |
| Boolean as 0 and 1 | `Bool` | `CASE c WHEN 1 THEN 'true' ELSE 'false' END AS c` |
| Date and time as `2026-10-30 12:00:00` | `Instant` | `strftime('%Y-%m-%dT%H:%M:%SZ', c) AS c` |
| Date as `2026-10-30` | `Date` | As it is |
| `BLOB` | `Bytes` | `'0x' \|\| hex(c) AS c` |

## Where adtb is not the right pick today

If you need one of these from SQLite or another SQL database, stay
there for now:

- Other programs or tools that talk to the database through SQL.
- Several processes on one file. [One process owns a database
  file](../concepts/serializable-transactions.md#concurrency-and-retry),
  and a second is refused. [SQLite lets processes on one host
  share a file](https://www.sqlite.org/wal.html).
- A host language other than Rust. The manual documents the shell and
  [the Rust API](../reference/rust-api.md).
- A dump, a backup or an export from the engine. `export`, `dump`,
  restore, backups, `compact` and `print schema` do not run
  yet, so there is no equivalent of `.dump`, `VACUUM` or `.schema`.
- Encryption, log shipping and embedded replicas, which the manual does
  not yet cover.
- Migration scripts. A change that needs a `script` is refused today.
- Views stored for speed. `store` is accepted, but reads still evaluate
  the query.
- Registering queries from the shell. `register bundle` runs only from
  a program.

The full list is [Not yet in this manual](../README.md#not-yet-in-this-manual).
