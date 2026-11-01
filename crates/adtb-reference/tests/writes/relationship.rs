use adtb_reference::Value;
use adtb_testkit::fixture::Fixture;

use super::created::{Created, User};
use super::shared::{Fields, bindings, finish, parse, take};

enum Invocation {
    Run(Vec<(String, String, Value)>),
    Call(Vec<Value>),
}

enum Probes {
    User(String),
    Google(String),
    Linked(String, String, Option<String>),
}

pub(super) struct Step {
    source: String,
    invocation: Invocation,
    created: Created,
    probes: Probes,
}

fn text(f: &Fixture, fields: &mut Fields, key: &str) -> Value {
    parse(f, "Text", &take(fields, key))
}

impl Step {
    pub(super) fn read(f: &Fixture, mode: &str, mut fields: Fields) -> Self {
        let source = take(&mut fields, "source");
        let invocation = match take(&mut fields, "invoke").as_str() {
            "run" => Invocation::Run(bindings(f, fields.remove("params"))),
            "call" => Invocation::Call(
                take(&mut fields, "args")
                    .lines()
                    .map(|arg| {
                        let (ty, value) = arg
                            .split_once('|')
                            .expect("an argument has a type and value");
                        parse(f, ty, value)
                    })
                    .collect(),
            ),
            other => panic!("unknown created-row invocation {other}"),
        };
        let (created, probes) = match mode {
            "created-user" => (
                Created::User(text(f, &mut fields, "name")),
                Probes::User(take(&mut fields, "query")),
            ),
            "created-google" => (
                Created::Google {
                    gid: text(f, &mut fields, "gid"),
                    user: User::Existing(parse(f, "Key users", &take(&mut fields, "user"))),
                },
                Probes::Google(take(&mut fields, "query")),
            ),
            "created-linked-google" | "created-linked-google-result" => (
                Created::Google {
                    gid: text(f, &mut fields, "gid"),
                    user: User::Created(text(f, &mut fields, "name")),
                },
                Probes::Linked(
                    take(&mut fields, "user-query"),
                    take(&mut fields, "reference-query"),
                    (mode == "created-linked-google-result")
                        .then(|| take(&mut fields, "identity-query")),
                ),
            ),
            _ => panic!("unknown created-row mode {mode}"),
        };
        finish(&fields);
        Self {
            source,
            invocation,
            created,
            probes,
        }
    }

    pub(super) fn run(self, f: &mut Fixture) {
        let before = f.db.clone();
        let out = match self.invocation {
            Invocation::Run(params) => {
                let borrowed: Vec<_> = params
                    .iter()
                    .map(|(n, t, v)| (n.as_str(), t.as_str(), v.clone()))
                    .collect();
                f.run_with(&self.source, &borrowed)
            }
            Invocation::Call(args) => f.call_txn(&self.source, args),
        }
        .expect("the created-row statement succeeds");
        let keys = self.created.check(f, &before, &out);
        match self.probes {
            Probes::User(query) => {
                let actual = f.one(&query);
                assert_eq!(out.result, actual, "original User result");
                assert_eq!(keys.user, actual, "created User native key witness");
            }
            Probes::Google(query) => {
                let actual = f.one(&query);
                assert_eq!(out.result, actual, "original Google result");
                assert_eq!(
                    keys.identity.expect("Google key"),
                    actual,
                    "created Google native key witness"
                );
            }
            Probes::Linked(user, reference, identity) => {
                if let Some(query) = identity {
                    let actual = f.one(&query);
                    assert_eq!(out.result, actual, "original Google result");
                    assert_eq!(
                        keys.identity.expect("Google key"),
                        actual,
                        "created Google native key witness"
                    );
                }
                let user = f.one(&user);
                let reference = f.one(&reference);
                assert_eq!(user, reference, "original User reference");
                assert_eq!(keys.user, user, "created User native key witness");
            }
        }
    }
}
