use adtb_reference::{Database, Outcome, Value};
use adtb_testkit::fixture::Fixture;

pub(super) enum User {
    Existing(Value),
    Created(Value),
}

pub(super) enum Created {
    User(Value),
    Google { gid: Value, user: User },
}

pub(super) struct Keys {
    pub(super) user: Value,
    pub(super) identity: Option<Value>,
}

enum Row {
    User,
    Google,
}

impl Row {
    fn fields(&self) -> (&str, &str, &str, &str) {
        match self {
            Self::User => ("users", "User", "User", "name"),
            Self::Google => ("identities", "Identity", "Google", "gid"),
        }
    }
}

fn inserted(f: &Fixture, before: &Database, out: &Outcome, row: &Row, payload: &Value) -> Value {
    let (relation, ty, ctor, field) = row.fields();
    let relation = f.rel(relation);
    let (_, ctor) = f.ctor(ty, ctor);
    let rows = out
        .change
        .inserted
        .get(&(relation, ctor))
        .expect("created rows exist");
    assert_eq!(1, rows.len(), "exactly one created row");
    let value = rows.first().expect("one inserted row");
    assert_eq!(Some(payload), value.field(field), "intended payload");
    let key = value.field("id").expect("the created row has an id");
    for (&(rel, _), rows) in before {
        if rel == relation {
            assert!(
                rows.iter().all(|old| old.field("id") != Some(key)),
                "fresh key across pre-state constructors"
            );
        }
    }
    assert!(
        out.db
            .get(&(relation, ctor))
            .is_some_and(|rows| rows.contains(value)),
        "exact inserted row in post-state"
    );
    value.clone()
}

fn user(f: &Fixture, before: &Database, out: &Outcome, name: &Value) -> Value {
    let row = inserted(f, before, out, &Row::User, name);
    row.field("id").expect("User id").clone()
}

impl Created {
    pub(super) fn check(self, f: &Fixture, before: &Database, out: &Outcome) -> Keys {
        match self {
            Self::User(name) => Keys {
                user: user(f, before, out, &name),
                identity: None,
            },
            Self::Google {
                gid,
                user: expected,
            } => {
                let user = match expected {
                    User::Existing(key) => key,
                    User::Created(name) => user(f, before, out, &name),
                };
                let row = inserted(f, before, out, &Row::Google, &gid);
                assert_eq!(
                    Some(&user),
                    row.field("user"),
                    "native Google.user references intended User"
                );
                Keys {
                    user,
                    identity: Some(row.field("id").expect("Google id").clone()),
                }
            }
        }
    }
}
