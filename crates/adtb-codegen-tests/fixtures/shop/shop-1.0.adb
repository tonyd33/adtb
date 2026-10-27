bundle shop-1.0 pins 2 (08618e7e1fd8626261883c11e14f29e52fc2b960d93d87f0930b9cf0fea999c0) edition "2026-10"
import shop
import fn method
fn label (c: Customer) -> Text = c.name
query names () -> Set Text = from c in customers select label(c)
query customer (k: Key customers) -> Set Customer =
  from c in customers where key c == k select c
query payments_of (k: Key customers) -> Set Payment =
  from p in payments where p.customer == k select p
query tiers () -> Set Tier = from c in customers select c.tier
query nicks () -> Set (Option Text) = from c in customers select c.nick
query summary () -> Set ({ name: Text, notes: Set Text, methods: Set (ByCard { last4: Text } | ByInvoice { due: I64 }) }) =
  from c in customers
  select {
    name: c.name,
    notes: from n in notes where n.customer == key c select n.body,
    methods: from p in payments where p.customer == key c
      select match p
        | Card x -> ByCard { last4: x.last4 }
        | Invoice i -> ByInvoice { due: i.due }
  }
query total () -> I64 = count(c for c in customers)
txn sign_up (n: Text, t: Tier, last4: Text) -> Key customers =
  transact
    let c = insert customers { name: n, tier: t, nick: None }
    insert payments.Card { customer: key c, last4: last4 }
    return key c
txn note (k: Key customers, body: Text) -> Key notes =
  transact
    let n = insert notes { customer: k, body: body }
    return key n
txn invoice (k: Key customers, due: I64) -> Key payments =
  transact
    let p = insert payments.Invoice { customer: k, due: due }
    return key p
txn set_nick (k: Key customers, nick: Option Text) -> Option Text =
  transact
    update c in customers where key c == k set Customer { c with nick: nick }
    return nick
query tags_of (k: Key customers) -> Set Label =
  from t in tags where t.customer == k select t
txn tag (k: Key customers, label: Text) -> Key tags =
  transact
    let t = insert tags { customer: k, label: label }
    return key t
