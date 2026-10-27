# The migration log

A database's schema is not a document you edit in place. It is the
result of an ordered list of migrations, the log, applied one after
another to an empty schema. Each migration is a file of operators that
you write: `add type`, `add field`, `rename constructor`, `enable` and
so on. The engine never compares two schemas and works out the
operators between them; the only way the schema changes is by a
migration you wrote, and the log keeps every one.

If you come from SQL migration tools, the log is the list of migration
scripts, but it is part of the database and checked by the engine:
the database knows which migrations it has applied, and refuses a log
that disagrees with it.

## Versions

Migrations are numbered from 1 without gaps, and the schema after
migration `n` is version `n`. The first migration of a database is
usually a schema file, a module of declarations, which the shell turns
into the operators that add each declaration. A program that opens the
database itself ships operators only, its first migration starting
with `add module`, as [the Rust API](../reference/rust-api.md#the-migrations)
shows. Save this as `shop.adt`:

```adt shop.adt
module shop

type Item { sku: Text, price: I64 } = Listed | Retired { why: Text }
relation items : Item key sku
```

```console
$ adtb --create --schema shop.adt shop.db
adtb> insert items.Listed { sku: "mug", price: 8 }
adtb> from m in catalog::migrations select { version: value(m.version), text: m.text }
{ text: "module shop\nadd module shop\nadd type Item { sku: Text, price: I64 } = Listed | Retired { why: Text }\nadd relation items : Item key sku\n", version: 1 }
adtb> \q
```

`catalog::migrations` holds the log. Migration 1 is the schema file
replayed as operators: `add module`, `add type`, `add relation`.

## The log only grows

Once a migration is applied, it is part of the database. Editing the
schema file afterwards does not change the database; it makes the file
disagree with the log, and the shell refuses it. Suppose you add a
`stock` field to `shop.adt` and save the result as `shop-edited.adt`:

```adt shop-edited.adt
module shop

type Item { sku: Text, price: I64, stock: I64 } = Listed | Retired { why: Text }
relation items : Item key sku
```

<!-- exit 1 -->
```console
$ adtb --schema shop-edited.adt shop.db -c 'from i in items select i'
shop.db: the file's migration 1 differs from the log's
```

To change a database that holds data, write the change as the next
migration. It says what to do with the rows that exist, here with a
`fill` for the new field:

```adt 0002-stock.adt
module shop
add field Item.stock: I64 fill 0
```

You give the shell the whole log, in order, one `--schema` per file.
It checks the migrations the database has against the files and
applies the rest. A log that is shorter than the database's is fine:
later runs need no `--schema` at all.

```console
$ adtb --schema shop.adt --schema 0002-stock.adt shop.db -c 'from i in items select i'
Listed { sku: "mug", price: 8, stock: 0 }
```

A migration applies as a whole or not at all. When one in a list is
refused, the ones before it stay applied and the database is at the
version of the last one that applied.

Undoing a migration does not remove it. `revert` appends a new
migration that applies the inverse of an earlier one, so the log still
records that the field existed:

```adt 0003-revert.adt
module shop
revert 0002
```

```console
$ adtb --schema shop.adt --schema 0002-stock.adt --schema 0003-revert.adt shop.db
adtb> from i in items select i
Listed { sku: "mug", price: 8 }
adtb> from m in catalog::migrations select value(m.version) order by value(m.version)
1
2
3
adtb> \q
```

## Readers first

A new constructor is a new kind of value that existing readers have
never seen. A program that matches on every constructor of `Item`
would meet a row it has no arm for. So adding a constructor and
letting rows of it be written are two steps. `add constructor` declares
it, and the constructor is closed to writes. A later migration
`enable`s it, once every reader knows about it:

```adt 0004-preorder.adt
module shop
add constructor Item.Preorder { ships: Text } after Listed
```

`after Listed` places the new constructor after `Listed` in the
type's order of constructors.

```adt 0005-enable.adt
module shop
enable items.Preorder
```

<!-- exit 1 0 -->
```console
$ adtb --schema shop.adt --schema 0002-stock.adt --schema 0003-revert.adt --schema 0004-preorder.adt shop.db
adtb> insert items.Preorder { sku: "lamp", price: 30, ships: "may" }
constructor `items.Preorder` is Declared, not open for writes
adtb> from i in items select match i | Listed _ -> "listed" | Retired _ -> "retired"
match does not cover `Preorder _`
adtb> \q
$ adtb --schema shop.adt --schema 0002-stock.adt --schema 0003-revert.adt --schema 0004-preorder.adt --schema 0005-enable.adt shop.db
adtb> insert items.Preorder { sku: "lamp", price: 30, ships: "may" }
adtb> from i in items select i order by i.sku
Preorder { sku: "lamp", price: 30, ships: "may" }
Listed { sku: "mug", price: 8 }
adtb> \q
```

From version 4 every query must already handle `Preorder`, as the
refused `match` shows, while no `Preorder` row can exist until version
5. `deprecate` closes an enabled constructor to new rows again, so
it can be emptied and later removed.

## Bundles and pinned versions

An application registers its queries and transactions in a bundle
pinned to a schema version, so that the engine checks each of them
once, at that version, and knows which constructors every live
program reads. A program registers its bundles when it opens the
database in process, as [the Rust API](../reference/rust-api.md)
shows; that open is also when it applies the migrations it ships. The
shell runs ad hoc text at the current version and registers no
bundles, and the `register bundle` command is refused:

<!-- exit 1 -->
```console
$ adtb shop.db -c 'register bundle "web.adb"'
`register bundle` does not run in this engine yet
```

A bundle pinned at an earlier version than the database's registers,
but this engine refuses its calls: it does not yet translate a call
across versions, so ship each bundle pinned at the version your log
ends at.

The rules are in [the operator log](../../spec/evolution.md#1-the-operator-log),
[versions](../../spec/evolution.md#2-versions),
[enable and deprecate](../../spec/evolution.md#43-enable-and-deprecate)
and [bundles](../../spec/applications.md#3-bundles). The
[migrations tutorial](../tutorial/7-migrations.md) walks through the
operators.
