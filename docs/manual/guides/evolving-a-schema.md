# Evolving a schema

A database that holds data changes shape only by migration. This guide
walks one small schema through a series of releases: adding and
renaming fields, adding a constructor and opening it to writes,
reshaping fields, undoing a migration and retiring a constructor. It
assumes you have read [Migrations](../tutorial/7-migrations.md) in the
tutorial; [The migration log](../concepts/migration-log.md) explains
the model behind it.

## The log is the list of files

A database's schema is the list of migrations applied to it, in order.
You keep that list as files, and you give the shell the whole list,
one `--schema` per file, oldest first. The shell checks the files the
database has already applied against its log and applies the rest, in
order, each as one transaction. Once a migration is applied, later
runs need no `--schema` at all.

Start with contacts that are either leads or customers. Save this as
`crm.adt`:

```adt crm.adt
module crm

type Contact { email: Text, name: Text }
  = Lead { source: Text }
  | Customer { since: I64 }
relation contacts : Contact key email

fn kind (c: Contact) -> Text = match c
  | Lead _ -> "lead"
  | Customer _ -> "customer"
```

```console
$ adtb --create --schema crm.adt crm.db
adtb> insert contacts.Lead { email: "ann@example.com", name: "ann", source: "web" }
adtb> insert contacts.Customer { email: "bo@example.com", name: "bo", since: 2021 }
adtb> \q
```

`kind` is a schema function: it lives in the schema, so the engine
rechecks it whenever a migration changes a type it reads.

## Add a field

A new field needs a value for the rows that already exist, given by
`fill`. Save this as `plan.adt`:

```adt plan.adt
module crm
add field Contact.Customer.plan: Text fill "basic"
```

```console
$ adtb --schema crm.adt --schema plan.adt crm.db -c 'from c in contacts.Customer select c'
Customer { email: "bo@example.com", name: "bo", since: 2021, plan: "basic" }
```

`fill` is evaluated once, at the migration. It is not a default: a
customer inserted later states its plan like any other field.
`Contact.Customer.plan` names a field of one constructor; `Contact.f`
would name a header field, which every constructor has.

## Rename a field

```adt channel.adt
module crm
rename field Contact.Lead.source to channel
```

<!-- exit 1 -->
```console
$ adtb --schema crm.adt --schema plan.adt --schema channel.adt crm.db
adtb> from c in contacts.Lead select c.source
Contact.Lead has no field `source`
adtb> from c in contacts.Lead select c.channel
"web"
adtb> \q
```

A rename keeps the data. Text that used the old name stops checking,
as the first query shows, and has to be updated to the new one. A
field is named as in the other field operators: `T.C.f` for a field
of constructor `C`, as `Contact.Lead.source` here, and `T.f` for a
header field or a field of a record type. In this engine a rename
ends the history of the relation for `at` reads
([Reading the past](reading-the-past.md#when-the-past-is-not-available)).

## Add a constructor, then open it

Partners are a third kind of contact. Adding a constructor to a sum
changes every `match` over that sum, so the migration that adds it
also gives each schema function its new arm, with `arm`. Save this as
`partner.adt`:

```adt partner.adt
module crm
add constructor Contact.Partner { company: Text }
  arm kind: Partner _ -> "partner"
```

Without the `arm` clause the migration is refused, because `kind`
would no longer cover every constructor.

A new constructor starts declared: it exists and can be read, but no
row can be written into it yet.

<!-- exit 1 -->
```console
$ adtb --schema crm.adt --schema plan.adt --schema channel.adt --schema partner.adt crm.db -c 'insert contacts.Partner { email: "cy@example.com", name: "cy", company: "acme" }'
constructor `contacts.Partner` is Declared, not open for writes
```

This is the readers-first rule. Ship the release of your application
that can read partners first, with its `match`es updated, and only
then open the constructor with `enable`, in a later migration:

```adt enable.adt
module crm
enable contacts.Partner
```

```console
$ adtb --schema crm.adt --schema plan.adt --schema channel.adt --schema partner.adt --schema enable.adt crm.db
adtb> insert contacts.Partner { email: "cy@example.com", name: "cy", company: "acme" }
adtb> from c in contacts select { name: c.name, kind: kind(c) } order by c.name
{ kind: "lead", name: "ann" }
{ kind: "customer", name: "bo" }
{ kind: "partner", name: "cy" }
adtb> \q
```

The full rules for the three write states, declared, enabled and
deprecated, are in
[enable and deprecate](../../spec/evolution.md#43-enable-and-deprecate).

## Move a field to the header

Only leads have a `channel`, but now every contact should record how
it arrived. `move field` moves it into the header; the constructors
that did not have it get a value from `fill`, written as a `match`
over their rows with no scrutinee:

```adt header.adt
module crm
move field Contact.Lead.channel to Contact fill match
  | Customer _ -> "unknown"
  | Partner _ -> "unknown"
```

```console
$ adtb --schema crm.adt --schema plan.adt --schema channel.adt --schema partner.adt --schema enable.adt --schema header.adt crm.db -c 'from c in contacts select { name: c.name, channel: c.channel } order by c.name'
{ channel: "web", name: "ann" }
{ channel: "unknown", name: "bo" }
{ channel: "unknown", name: "cy" }
```

A header field is read on the whole relation, without a `match`.

## Undo a migration

Every operator has an inverse, and `revert` applies the inverse of an
earlier migration as a new one. The log only grows: the reverted
migration stays in it, and the revert is the next entry. Migration
numbers are written with four digits; `plan.adt` was migration 2:

```adt revert.adt
module crm
revert 0002
```

```console
$ adtb --schema crm.adt --schema plan.adt --schema channel.adt --schema partner.adt --schema enable.adt --schema header.adt --schema revert.adt crm.db -c 'from c in contacts.Customer select c'
Customer { email: "bo@example.com", name: "bo", channel: "unknown", since: 2021 }
```

The inverse of `add field` drops the field, so the plans are gone.
Some inverses lose data like this; the specification lists which in
[inverses and revert](../../spec/evolution.md#5-inverses-and-revert).

## Retire a constructor

A constructor goes away in two steps, each its own migration. First
`deprecate` it, which closes it to new rows while existing rows can
still be read, updated and deleted:

```adt deprecate.adt
module crm
deprecate contacts.Partner
```

<!-- exit 1 -->
```console
$ adtb --schema crm.adt --schema plan.adt --schema channel.adt --schema partner.adt --schema enable.adt --schema header.adt --schema revert.adt --schema deprecate.adt crm.db -c 'insert contacts.Partner { email: "dee@example.com", name: "dee", channel: "web", company: "acme" }'
constructor `contacts.Partner` is Deprecated, not open for writes
```

Then, once its rows are gone, `remove` it. `remove` never deletes
data: it is refused while the constructor holds rows. Every schema
function that names the constructor is redefined in the same
migration:

```adt remove.adt
module crm
remove constructor Contact.Partner
redefine fn kind (c: Contact) -> Text = match c
  | Lead _ -> "lead"
  | Customer _ -> "customer"
```

```console
$ adtb crm.db -c 'delete c in contacts.Partner'
$ adtb --schema crm.adt --schema plan.adt --schema channel.adt --schema partner.adt --schema enable.adt --schema header.adt --schema revert.adt --schema deprecate.adt --schema remove.adt crm.db -c 'from c in contacts select { name: c.name, kind: kind(c) } order by c.name'
{ kind: "lead", name: "ann" }
{ kind: "customer", name: "bo" }
```

Instead of deleting the rows you can keep them by moving them to
another constructor first, with an update in the application's own
release; [Modelling with sums](modelling-with-sums.md#move-a-row-between-constructors)
shows the retag.

## Split and merge constructors

When one constructor turns out to be two kinds of thing, `split`
replaces it with several, routing each existing row with a `match`.
`merge` is the inverse: it folds several constructors into one whose
discriminator field records which one each row came from. A merge
needs an enumeration whose constructors are exactly the merged ones,
for that field. These operators are shown in a history of their own,
on a fresh database:

```adt orders.adt
module shop

type Order { id: I64, total: I64 }
  = Placed
  | Shipped { carrier: Text }
relation orders : Order key id
```

```adt split.adt
module shop
split constructor Order.Placed into Small | Large
  by match
    | Placed p if p.total < 100 -> Small {}
    | Placed p -> Large {}
```

```adt merge.adt
module shop
add type Size = Small | Large
merge constructor Order.(Small | Large) into Placed { size: Size }
```

```console
$ adtb --create --schema orders.adt orders.db
adtb> insert orders.Placed { id: 1, total: 40 }
adtb> insert orders.Placed { id: 2, total: 400 }
adtb> \q
$ adtb --schema orders.adt --schema split.adt orders.db -c 'from o in orders select o order by o.id'
Small { id: 1, total: 40 }
Large { id: 2, total: 400 }
$ adtb --schema orders.adt --schema split.adt --schema merge.adt orders.db -c 'from o in orders select o order by o.id'
Placed { id: 1, total: 40, size: Small }
Placed { id: 2, total: 400, size: Large }
```

The arms of the split's `match` may leave out the key and header
fields, which carry over from the row. When a schema function matches
on the constructor being split or merged, the operator also takes an
`arm` clause for each constructor it produces, as `add constructor`
does. The specification describes
[split](../../spec/evolution.md#46-split-a-constructor),
[merge](../../spec/evolution.md#47-merge-constructors) and the
[nest and unnest](../../spec/evolution.md#48-nest-and-unnest) pair,
which move constructors into and out of a value-level sum field.

## Defaults and computed fields

`add default` fills an omitted field on later inserts. It leaves
existing rows alone; `fill` supplies their values when the field is
added. `add computed` adds a field evaluated from each row on read.
Here is a separate history showing all three:

```adt items.adt
module inventory

type Item = { id: I64, price: I64 }
relation items : Item key id
```

```console
$ adtb --create --schema items.adt items.db -c 'insert items { id: 1, price: 10 }'
```

```adt derived.adt
module inventory
add field Item.tax: I64 fill 2
add default items.tax = 3
add computed items.total: I64 = self.price + self.tax
```

```console
$ adtb --schema items.adt --schema derived.adt items.db
adtb> insert items { id: 2, price: 20 }
adtb> insert items { id: 3, price: 30, tax: 4 }
adtb> from i in items select { id: i.id, tax: i.tax, total: i.total } order by i.id
{ id: 1, tax: 2, total: 12 }
{ id: 2, tax: 3, total: 23 }
{ id: 3, tax: 4, total: 34 }
adtb> update i in items where i.id == 2 set { i with price: 21 }
adtb> from i in items where i.id == 2 select i.total
24
adtb> \q
```

The first row keeps its fill of `2`. The second takes the default of
`3`, and the third supplies its own tax. `total` uses the current row:
it changes when `price` changes and is never supplied in an insert.
Defaults belong to the relation (`items.tax`); the physical field
belongs to the type (`Item.tax`). `items.Item.tax` would give a
default for only that constructor of the relation.

Use `drop default items.tax` to require the field again on later
inserts, `redefine computed items.total: I64 = ...` to change the
expression, or `drop computed items.total` to remove the computed
field.

## What the engine refuses at apply

A migration is checked against the data before it applies, and a
refused migration changes nothing. The shell prints why and exits
without running its text. Some refusals you will meet:

```adt unique.adt
module shop
add unique (total) in orders
```

<!-- exit 0 1 -->
```console
$ adtb orders.db -c 'insert orders.Shipped { id: 3, total: 400, carrier: "post" }'
$ adtb --schema orders.adt --schema split.adt --schema merge.adt --schema unique.adt orders.db -c 'from o in orders select o.id'
orders.db: migration 4 is refused: rows of orders share the key 400 of `unique (total)`
```

A unique added over data that breaks it is refused, as is a
constraint the data violates; fix the data first, then add it. Others
you will see:

- Disabling a constructor that holds rows (`disable r.C` returns an
  enabled constructor with no rows to declared): "constructor relation
  contacts.Lead holds rows; it cannot be disabled".
- A schema function that no longer checks after the migration, such
  as one reading a dropped field: the refusal names the function's
  error. Redefine the function in the same migration.
- A file you edited after it was applied: "the file's migration 2
  differs from the log's". Applied migrations never change; write a
  new one.

The `script` operator is intended to run a write inside a migration,
but the engine refuses it with "the engine does not run a `script`
yet".
