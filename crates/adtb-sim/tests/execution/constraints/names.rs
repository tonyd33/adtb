use adtb::{CallOptions, Database, Error, Log, OpenOptions, RejectReason, Value};
use adtb_lang::check::check_schema;
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::textual::{Form, parse};
use adtb_reference::{Abort, Rejection, Value as RefValue};
use adtb_simkit::api;
use adtb_simkit::open;
use adtb_testkit::fixture::Fixture;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

struct Case {
    name: String,
    fields: BTreeMap<String, String>,
}

impl Case {
    fn get(&self, key: &str) -> &str {
        self.fields
            .get(key)
            .unwrap_or_else(|| panic!("{} lacks {key}", self.name))
    }
}

fn cases() -> Vec<Case> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/constraints/unique-names");
    let mut paths: Vec<_> = std::fs::read_dir(root)
        .expect("name cases")
        .map(|p| p.expect("case entry").path())
        .collect();
    paths.sort();
    assert_eq!(
        paths.len(),
        31,
        "all constructor orders and distinct naming contracts"
    );
    let mut names = BTreeSet::new();
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .expect("case name")
                .to_str()
                .expect("UTF-8 name")
                .to_owned();
            assert!(names.insert(name.clone()), "duplicate case name");
            let text = std::fs::read_to_string(path).expect("runtime naming case");
            assert!(text.starts_with('@'));
            let mut fields = BTreeMap::new();
            for field in text[1..].trim_end().split("\n@") {
                let (key, value) = field.split_once('\n').unwrap_or((field, ""));
                assert!(
                    fields.insert(key.to_owned(), value.to_owned()).is_none(),
                    "duplicate {key}"
                );
            }
            assert_eq!(
                fields.keys().map(String::as_str).collect::<Vec<_>>(),
                [
                    "before",
                    "catalog",
                    "constraint",
                    "decls",
                    "entry",
                    "report",
                    "seed",
                    "write"
                ]
            );
            Case { name, fields }
        })
        .collect()
}

fn reference_fixture(case: &Case) -> Fixture {
    if case.get("entry") == "reference" {
        return Fixture::new(case.get("decls"));
    }
    assert_eq!(case.get("entry"), "engine");
    let source = adtb_lang::signature::replay([case.get("decls")]).expect("case log applies");
    let schema = source.schema().expect("case schema");
    Fixture {
        program: check_schema(&schema).expect("case checks"),
        schema,
        db: BTreeMap::new(),
        supply: Default::default(),
        closed: BTreeMap::new(),
        files: Default::default(),
    }
}

fn form(schema: &Schema) -> Form {
    let module = schema.module_named("app").expect("case module");
    let ValueDef::Relation(rel) = schema.module(module).values["rows"] else {
        panic!("rows is a relation")
    };
    Form::of(schema, &schema.relation(rel).elem)
}

fn state(db: &Database<adtb_sim::SimRuntime>) -> Vec<Value> {
    let mut out: Vec<_> = api::result(db, "from r in rows select r")
        .map(|v| v.expect("state row"))
        .collect();
    out.sort();
    out
}

fn reference(case: &Case, f: &mut Fixture, constraint: &str) {
    let ty = f.ty("Row");
    let read = |text: &str| {
        adtb_reference::textual::parse(&f.schema, text, &ty).expect("external reference value")
    };
    let before = RefValue::set(case.get("before").lines().map(read));
    let report = case
        .get("report")
        .lines()
        .map(|line| {
            let (a, b) = line.split_once('~').expect("external witness pair");
            RefValue::record([("a", read(a)), ("b", read(b))])
        })
        .collect();
    f.run_with(case.get("seed"), &[]).expect("reference seed");
    assert_eq!(
        f.eval("from r in rows select r"),
        Ok(before.clone()),
        "{} reference before",
        case.name
    );
    assert_eq!(
        f.run_with(case.get("write"), &[]).map(|_| ()),
        Err(Abort::Reject(Rejection::Violated {
            constraint: constraint.to_owned(),
            report
        })),
        "{} reference refusal",
        case.name
    );
    assert_eq!(
        f.eval("from r in rows select r"),
        Ok(before),
        "{} reference unchanged",
        case.name
    );
}

#[test]
fn canonical_unique_names_cover_constructor_orders_scopes_paths_and_ids() {
    let mut reached = BTreeMap::new();
    for case in cases() {
        let mut fixture = reference_fixture(&case);
        if case.get("entry") == "reference" {
            assert!(case.get("catalog").is_empty());
            reference(&case, &mut fixture, case.get("constraint"));
            *reached.entry("named").or_insert(0usize) += 1;
            continue;
        }
        api::once(0, |rt| {
            let log = Log::new([case.get("decls")]);
            let db = Database::open_in(rt.clone(), "names.adt", &log, &open::options(4))
                .expect("public open");
            let row_form = form(db.schema());
            let read = |text: &str| parse::<Value>(text, &row_form).expect("external engine value");
            let mut before: Vec<_> = case.get("before").lines().map(read).collect();
            before.sort();
            let mut report: Vec<_> = case
                .get("report")
                .lines()
                .map(|line| {
                    let (a, b) = line.split_once('~').expect("external witness pair");
                    Value::record(vec![read(a), read(b)])
                })
                .collect();
            report.sort();
            let constraint = if case.get("catalog").is_empty() {
                case.get("constraint").to_owned()
            } else {
                let ids: Vec<_> = api::result(&db, case.get("catalog"))
                    .map(|v| v.expect("catalog identity"))
                    .collect();
                let [Value::Int(id)] = ids.as_slice() else {
                    panic!("one public catalog index id: {ids:?}")
                };
                assert!(*id > 1024, "catalog identity is not a local index position");
                case.get("constraint").replace("{catalog}", &id.to_string())
            };
            db.query_text(case.get("seed"), &[], &CallOptions::default())
                .expect("public seed");
            assert_eq!(state(&db), before, "{} engine before", case.name);
            let mut got = db
                .query_text(case.get("write"), &[], &CallOptions::default())
                .map(|_| ());
            if let Err(Error::Rejected {
                reason: RejectReason::Violated { report, .. },
            }) = &mut got
            {
                report.sort();
            }
            assert_eq!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::Violated {
                        constraint: constraint.clone(),
                        report,
                        redacted: 0
                    }
                }),
                "{} public refusal",
                case.name
            );
            assert_eq!(state(&db), before, "{} engine unchanged", case.name);
            db.close().expect("close public database");
            let db = Database::open_in(rt, "names.adt", &log, &OpenOptions::default())
                .expect("reopen public database");
            assert_eq!(state(&db), before, "{} reopened state", case.name);
            db.close().expect("close reopened database");
            reference(&case, &mut fixture, &constraint);
        });
        *reached.entry("public").or_insert(0usize) += 1;
    }
    assert_eq!(reached, [("named", 2), ("public", 29)].into());
}
