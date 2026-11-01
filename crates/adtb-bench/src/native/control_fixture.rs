use super::super::model::Snapshot;
use crate::native::schema;
use adtb::Value;
use adtb::typed::HostValue;
use adtb_lang::schema::Schema;
use adtb_lang::textual::{self, Form};
use adtb_lang::types::RelId;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub fn read(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/controls")
            .join(name),
    )
    .expect("read runtime schema control fixture")
}

pub struct Case {
    pub fields: BTreeMap<String, String>,
}

impl Case {
    pub fn field(&self, name: &str) -> &str {
        self.fields
            .get(name)
            .unwrap_or_else(|| panic!("missing {name}"))
    }
}

pub fn cases(name: &str) -> Vec<Case> {
    let mut names = BTreeSet::new();
    read(name)
        .split("\n---\n")
        .map(|block| {
            let mut fields = BTreeMap::new();
            let mut key = None;
            for line in block.lines() {
                if let Some(k) = line.strip_prefix('@') {
                    assert!(
                        fields.insert(k.to_owned(), String::new()).is_none(),
                        "duplicate {k}"
                    );
                    key = Some(k.to_owned());
                } else if let Some(k) = &key {
                    let value = fields.get_mut(k).expect("section exists");
                    if !value.is_empty() {
                        value.push('\n');
                    }
                    value.push_str(line);
                } else {
                    assert!(line.is_empty(), "text precedes a fixture section");
                }
            }
            let case = Case { fields };
            assert!(
                names.insert(case.field("name").to_owned()),
                "duplicate case"
            );
            let actual: BTreeSet<_> = case.fields.keys().map(String::as_str).collect();
            assert_eq!(
                actual,
                [
                    "party", "name", "family", "setup", "after", "adtb", "sqlite", "native", "sql",
                    "report"
                ]
                .into()
            );
            if !case.field("native").starts_with("violation|") {
                assert!(case.field("report").is_empty(), "unused refusal report");
            }
            case
        })
        .collect()
}

pub fn relation(schema: &Schema, name: &str) -> RelId {
    let module = schema
        .modules
        .iter()
        .find(|m| m.name == "baseline")
        .expect("baseline module");
    match module.values.get(name).expect("declared fixture relation") {
        adtb_lang::schema::ValueDef::Relation(id) => *id,
        other => panic!("expected relation, found {other:?}"),
    }
}

pub fn value(schema: &Schema, table: &str, text: &str) -> Value {
    let id = relation(schema, table);
    textual::parse(text, &Form::of(schema, &schema.relation(id).elem))
        .unwrap_or_else(|e| panic!("{table}: {text}: {e:?}"))
}

pub fn report(schema: &Schema, text: &str) -> Vec<Value> {
    let mut rows: Vec<_> = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (table, row) = line.split_once('|').expect("report relation|row");
            if let Some((left, right)) = row.split_once('~') {
                Value::record(vec![
                    value(schema, table, left),
                    value(schema, table, right),
                ])
            } else {
                value(schema, table, row)
            }
        })
        .collect();
    rows.sort();
    rows
}

pub fn snapshot(schema: &Schema, name: &str) -> Snapshot {
    let mut out = Snapshot {
        counters: Vec::new(),
        campaigns: Vec::new(),
        entities: Vec::new(),
        sessions: Vec::new(),
    };
    for line in read(name).lines() {
        let (table, row) = line.split_once('|').expect("snapshot relation|row");
        let row = value(schema, table, row);
        match table {
            "counters" => out
                .counters
                .push(schema::Counter::from_value(&row).expect("Counter")),
            "campaigns" => out
                .campaigns
                .push(schema::Campaign::from_value(&row).expect("Campaign")),
            "entities" => out
                .entities
                .push(schema::Entity::from_value(&row).expect("Entity")),
            "sessions" => out
                .sessions
                .push(schema::Session::from_value(&row).expect("Session")),
            other => panic!("unknown snapshot table {other}"),
        }
    }
    out.sort();
    out
}
