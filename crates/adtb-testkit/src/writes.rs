use adtb_reference::Value;

use crate::fixture::Fixture;
use crate::uuid;

/// The schema of the write tests, a module file without its `module` line.
pub const SCHEMA: &str = r#"
type User = { id: Key users, name: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
relation users : User key id generate sequence
relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
  unique (sub) in Apple
  unique (lower(email)) in EmailPass
set OAuth of identities = Google | Apple
constraint login_present =
  from i in identities
  where match i
    | EmailPass e -> scalar_len(e.email) == 0
    | Google g    -> scalar_len(g.gid) == 0
    | Apple a     -> scalar_len(a.sub) == 0
  select i

type Login { id: Key logins, user: Key users on delete cascade }
  = Active  { identity: Key identities on delete retag Revoked {} }
  | Revoked { at: Instant }
relation logins : Login
  key id generate sequence
  default Revoked.at = generate clock

type Reset = { id: Key resets, identity: Key identities.EmailPass on delete cascade, token: Bytes }
relation resets : Reset key id generate sequence

type Badge = { id: Key badges, owner: Key users on delete restrict, label: Text }
relation badges : Badge key id generate sequence

type Import { id: Key imports } = Email { user: Key users, addr: Text, h: Bytes }
                                | Oauth { user: Key users, sub: Text }
relation imports : Import key id generate sequence

type Note = { id: Key notes, author: Option (Key users) on delete clear, body: Text }
relation notes : Note key id generate sequence

fn lookup (email: Text) -> Text = lower(email)
fn rehash (h: Bytes) -> Bytes = h

type Address = US { zip: Text, street: Text } | Intl { country: Text, postcode: Text }
type Customer = { id: Key customers, name: Text, address: Address }
relation customers : Customer key id generate sequence

type Ticket { id: Key tickets, owner: Option (Key users) on delete clear }
  = Open { identity: Key identities on delete retag Closed {} }
  | Closed
relation tickets : Ticket key id generate sequence

type Raw = { id: Key raws, n: I64 }
relation raws : Raw key id generate sequence
relation user_rejects : RejectedRow key (load, line)

type Email = new Text where scalar_len(self) > 3
type Contact = { id: Key contacts, email: Email }
relation contacts : Contact key id generate sequence

constraint aa_names = from u in users where u.name == "aa" select u
constraint zz_names = from u in users where u.name == "zz" select u
constraint frozen_notes = from n in deleted(notes) where n.body == "frozen" select n
check name_short on users (scalar_len(name) < 20)
check divides on raws (10 / n != 0)
"#;

/// Returns the write schema holding its sample rows.
pub fn fixture() -> Fixture {
    let mut f = Fixture::new(SCHEMA);
    f.rows(
        "users",
        r#"[User { id: users.key(1), name: "ann" },
            User { id: users.key(2), name: "bo" },
            User { id: users.key(3), name: "cy" },
            User { id: users.key(4), name: "dee" }]"#,
    );
    f.rows(
        "identities",
        &format!(
            r#"[EmailPass {{ id: {}, user: users.key(1), email: "ann@example.com", hash: bytes("h1") }},
                Google {{ id: {}, user: users.key(1), gid: "g-ann" }},
                Google {{ id: {}, user: users.key(2), gid: "g-bo" }},
                Google {{ id: {}, user: users.key(1), gid: "s-bo" }},
                EmailPass {{ id: {}, user: users.key(2), email: "Bo@Example.com", hash: bytes("h2") }}]"#,
            uuid(1),
            uuid(2),
            uuid(3),
            uuid(4),
            uuid(5)
        ),
    );
    f.rows_with(
        "logins",
        &format!(
            r"[Active {{ id: logins.key(1), user: users.key(1), identity: {} }},
                Active {{ id: logins.key(2), user: users.key(2), identity: {} }},
                Revoked {{ id: logins.key(3), user: users.key(1), at: t }}]",
            uuid(1),
            uuid(3)
        ),
        &[("t", "Instant", Value::Instant(5))],
    );
    f.rows(
        "resets",
        r#"from e in identities.EmailPass where e.email == "ann@example.com"
           select Reset { id: resets.key(1), identity: key e, token: bytes("t") }"#,
    );
    f.rows(
        "badges",
        r#"[Badge { id: badges.key(1), owner: users.key(3), label: "b3" },
            Badge { id: badges.key(2), owner: users.key(4), label: "b4" }]"#,
    );
    f.rows(
        "imports",
        r#"[Oauth { id: imports.key(1), user: users.key(2), sub: "s-bo" },
            Email { id: imports.key(2), user: users.key(3), addr: "cy@x.org", h: bytes("h3") }]"#,
    );
    f.rows(
        "notes",
        r#"[Note { id: notes.key(1), author: Some { value: users.key(4) }, body: "n" },
            Note { id: notes.key(2), author: None, body: "plain" },
            Note { id: notes.key(3), author: None, body: "frozen" }]"#,
    );
    f.rows(
        "customers",
        r#"[Customer { id: customers.key(1), name: "c", address: US { zip: "10001", street: "s" } },
            Customer { id: customers.key(2), name: "d", address: Intl { country: "fr", postcode: "75001" } }]"#,
    );
    f.rows(
        "tickets",
        &format!(
            r"[Open {{ id: tickets.key(1), owner: Some {{ value: users.key(4) }}, identity: {} }}]",
            uuid(2)
        ),
    );
    f.rows(
        "raws",
        "[Raw { id: raws.key(1), n: 5 }, Raw { id: raws.key(2), n: -1 }, Raw { id: raws.key(3), n: 7 }]",
    );
    f
}
