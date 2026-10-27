# No NULL

adtb has no NULL. Every field of a row holds a value of its type, every
time. A fact that may be absent is said so in the type, with a
constructor for the absent case, and a query that reads it says what to
do in each case.

In SQL, NULL stands for several different things at once: unknown, not
applicable, not yet set, no partner in an outer join. It also changes
how comparisons work: `phone <> '555'` is neither true nor false for a
row whose phone is NULL, so that row silently drops out of both a
query and its negation. adtb replaces each of these uses with something
you can see in the schema.

## A field a row does not have

The first use of NULL is a column that only some kinds of row fill. In
adtb that is a field only some constructors declare, as
[Relations are sums](relations-as-sums.md) shows: a row of a
constructor without the field does not have it, and reading it there
is a type error, not an empty value.

## Absence in the type

When one kind of row may or may not have a value, the field's type
says so. The prelude's `Option` is a sum with two constructors, `None`
and `Some { value: a }`. It is an ordinary sum, defined in the library,
not a special marker. When the absent case means something specific,
give it a name of its own with a sum you declare, as `Delivery` does
here. Save this as `crm.adt`:

```adt crm.adt
module crm

type Person = { email: Text, name: Text, phone: Option Text }
relation people : Person key email

type Coupon = { code: Text, pct: I64 }
relation coupons : Coupon key code

type Delivery = Pickup | Ship { address: Text }

type Order = { id: I64, buyer: Key people, coupon: Option (Key coupons) on delete clear, delivery: Delivery }
relation orders : Order key id
```

An insert gives every field. There is no implicit NULL for a field you
leave out, and `null` is not a word of the language:

```console
$ adtb --create --schema crm.adt crm.db
adtb> insert people { email: "ann@x", name: "Ann", phone: Some { value: "555" } }
adtb> insert people { email: "bo@x", name: "Bo", phone: None }
adtb> insert people { email: "cy@x", name: "Cy" }
field `phone` of `Person` is missing, and `people` has no default for it
adtb> insert coupons { code: "TEN", pct: 10 }
adtb> insert orders { id: 1, buyer: people.key("ann@x"), coupon: Some { value: coupons.key("TEN") }, delivery: Pickup }
adtb> insert orders { id: 2, buyer: people.key("ann@x"), coupon: None, delivery: Ship { address: "1 Main St" } }
```

## Comparisons are two-valued

`None` is a value like any other, and `==` and `!=` are always true or
false. A person with no phone is a person whose phone is not
`Some { value: "555" }`:

```console
$ adtb crm.db
adtb> from p in people where p.phone == Some { value: "555" } select p.name
"Ann"
adtb> from p in people where p.phone != Some { value: "555" } select p.name
"Bo"
adtb> from p in people where p.phone == None select p.name
"Bo"
```

The first two queries partition the relation: every person is in
exactly one of them.

## Reading an optional value

A path stops at an `Option`. To use the value inside, `match` on it and
say what the `None` case gives:

```console
$ adtb crm.db
adtb> from p in people select p.phone.value
Option Text has no field `value`; a path stops at an `Option`: match it
adtb> from p in people select { name: p.name, phone: match p.phone | Some s -> s.value | None -> "none" } order by p.name
{ name: "Ann", phone: "555" }
{ name: "Bo", phone: "none" }
adtb> from o in orders select { id: o.id, off: match o.coupon | Some { value: c } -> c.pct | None -> 0 } order by o.id
{ id: 1, off: 10 }
{ id: 2, off: 0 }
adtb> from o in orders select { id: o.id, to: match o.delivery | Pickup -> "counter" | Ship s -> s.address } order by o.id
{ id: 1, to: "counter" }
{ id: 2, to: "1 Main St" }
```

Because a `match` must cover every constructor
([Tutorial 4](../tutorial/4-match.md)), the absent case is never
forgotten: leaving out `None` is an error, not a NULL that flows on.

## The missing side of a join

SQL's outer join pads the missing side with NULLs. In adtb a join is a
comprehension over two ranges, and a row with no partner is simply not
in the result. When you want every row with its partners, possibly
none, nest the partners as a set:

```console
$ adtb crm.db
adtb> from p in people, o in orders where o.buyer == key p select { name: p.name, order: o.id } order by o.id
{ name: "Ann", order: 1 }
{ name: "Ann", order: 2 }
adtb> from p in people select { name: p.name, orders: from o in orders where o.buyer == key p select o.id } order by p.name
{ name: "Ann", orders: [1, 2] }
{ name: "Bo", orders: [] }
```

Bo has no orders, and his set of orders is empty: `[]`, not NULL.

## Aggregates over nothing

An aggregate over an empty range has a defined value. `count` and
`sum` give 0. `min` and `max` have no value to give, so they return an
`Option`:

```console
$ adtb crm.db
adtb> count(o for o in orders where o.id > 10)
0
adtb> sum(o.id for o in orders where o.id > 10)
0
adtb> max(o.id for o in orders where o.id > 10)
None
adtb> max(o.id for o in orders)
Some { value: 2 }
```

## Optional references

A reference that may be absent is an `Option (Key ...)` field. Its
delete action `on delete clear` sets it to `None` when the target row
is deleted:

```console
$ adtb crm.db
adtb> delete c in coupons where c.code == "TEN"
adtb> from o in orders select { id: o.id, coupon: o.coupon } order by o.id
{ coupon: None, id: 1 }
{ coupon: None, id: 2 }
```

The rules are in [the model](../../spec/model.md#12-what-values-are-not)
and [match](../../spec/queries.md#57-no-null).
