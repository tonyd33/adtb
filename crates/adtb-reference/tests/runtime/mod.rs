use std::collections::BTreeMap;
use std::path::Path;

use adtb_reference::{Value, textual};
use adtb_testkit::fixture::Fixture;

pub(super) type Fields = BTreeMap<String, String>;

pub(super) fn fields(block: &str) -> (&str, &str, Fields) {
    let (header, body) = block.split_once('\n').expect("a case has a body");
    let (name, mode) = header.split_once(' ').expect("a case names its mode");
    assert!(!name.is_empty() && !name.contains('/'), "invalid case name");
    assert!(body.starts_with("--- "), "a case starts with a field");
    let mut fields = BTreeMap::new();
    for field in body[4..].trim_end().split("\n--- ") {
        let (key, value) = field.split_once('\n').unwrap_or((field, ""));
        assert!(
            fields.insert(key.into(), value.trim_end().into()).is_none(),
            "duplicate field {key}"
        );
    }
    (name, mode, fields)
}

pub(super) fn take(fields: &mut Fields, key: &str) -> String {
    fields
        .remove(key)
        .unwrap_or_else(|| panic!("missing field {key}"))
}

pub(super) fn finish(fields: &Fields) {
    assert!(
        fields.is_empty(),
        "unknown or unused fields: {:?}",
        fields.keys()
    );
}

pub(super) fn parse(fixture: &Fixture, ty: &str, text: &str) -> Value {
    textual::parse(&fixture.schema, text, &fixture.ty(ty))
        .unwrap_or_else(|error| panic!("expected/input {ty} `{text}`: {error}"))
}

pub(super) fn bindings(fixture: &Fixture, text: Option<String>) -> Vec<(String, String, Value)> {
    let mut names = std::collections::BTreeSet::new();
    text.into_iter()
        .flat_map(|s| s.lines().map(str::to_owned).collect::<Vec<_>>())
        .map(|line| {
            let mut parts = line.splitn(3, '|');
            let name = parts.next().expect("binding has a name").to_string();
            assert!(
                !name.is_empty() && names.insert(name.clone()),
                "binding names are nonempty and unique"
            );
            let ty = parts.next().expect("binding has a type").to_string();
            let value = parse(fixture, &ty, parts.next().expect("binding has a value"));
            (name, ty, value)
        })
        .collect()
}

pub(super) fn read(directory: &Path, name: &str) -> String {
    let path = directory.join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

pub(super) fn fixture(directory: &Path) -> Fixture {
    let mut fixture = Fixture::new(&read(directory, "schema.adtb"));
    let data = read(directory, "data.txt");
    assert!(data.starts_with("=== "), "setup declares rows");
    for block in data[4..].split("\n=== ") {
        let (relation, mode, mut fields) = fields(block);
        assert_eq!(mode, "rows", "setup only loads rows");
        let source = take(&mut fields, "source");
        let params = bindings(&fixture, fields.remove("params"));
        let borrowed: Vec<_> = params
            .iter()
            .map(|(n, t, v)| (n.as_str(), t.as_str(), v.clone()))
            .collect();
        fixture.rows_with(relation, &source, &borrowed);
        finish(&fields);
    }
    fixture
}

pub(super) fn message(error: &(dyn std::any::Any + Send)) -> &str {
    error
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| error.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic")
}
