use super::*;
use std::collections::BTreeSet;
use std::path::Path;

struct Case {
    name: String,
    mode: String,
    fields: BTreeMap<String, String>,
}

struct Outcome {
    phase: &'static str,
    errors: Option<Vec<String>>,
    ty: Option<String>,
}

fn directory() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/checker"))
}

fn parse(name: &str, text: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    for block in text.split("=== ").skip(1) {
        let block = block
            .strip_suffix('\n')
            .expect("a case ends with a newline");
        let (header, body) = block.split_once('\n').expect("a case header has a newline");
        let (id, mode) = header
            .split_once(' ')
            .expect("a case header has id and mode");
        assert!(
            matches!(mode, "schema" | "adhoc" | "query"),
            "unknown mode {mode}"
        );
        assert!(!id.is_empty() && !id.contains('/'), "invalid case id {id}");
        assert!(body.starts_with("--- "), "a case starts with a field");
        let mut fields = BTreeMap::new();
        for field in body[4..].split("\n--- ") {
            let (key, value) = field.split_once('\n').expect("a field has a value");
            assert!(
                fields.insert(key.to_string(), value.to_string()).is_none(),
                "duplicate field {key}"
            );
        }
        cases.push(Case {
            name: format!("{name}/{id}"),
            mode: mode.to_string(),
            fields,
        });
    }
    assert!(
        text.starts_with("=== ") && !cases.is_empty(),
        "a fixture declares cases"
    );
    cases
}

fn load() -> (Vec<Case>, Vec<String>) {
    let mut cases = Vec::new();
    let mut failures = Vec::new();
    let mut paths: Vec<_> = std::fs::read_dir(directory())
        .expect("the checker fixtures are readable")
        .map(|e| e.expect("a fixture entry is readable").path())
        .filter(|p| p.extension().is_some_and(|e| e == "case"))
        .collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_stem()
            .expect("a fixture has a stem")
            .to_str()
            .expect("UTF-8 fixture name");
        let result = std::panic::catch_unwind(|| {
            let text = std::fs::read_to_string(&path).expect("the fixture is readable");
            parse(name, &text)
        });
        match result {
            Ok(mut loaded) => cases.append(&mut loaded),
            Err(e) => failures.push(format!("{}: {}", path.display(), panic_message(e.as_ref()))),
        }
    }
    (cases, failures)
}

fn panic_message(e: &(dyn std::any::Any + Send)) -> &str {
    e.downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| e.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic")
}

fn schema_outcome(text: &str) -> Outcome {
    let result = match schema(text) {
        Ok(s) => check_program(s),
        Err(errors) => {
            return Outcome {
                phase: "schema",
                errors: Some(errors),
                ty: None,
            };
        }
    };
    Outcome {
        phase: "check",
        errors: result.err(),
        ty: None,
    }
}

fn outcome(mode: &str, text: &str, fields: &mut BTreeMap<String, String>, typed: bool) -> Outcome {
    match mode {
        "schema" => {
            assert!(!typed, "only ad hoc cases have a type expectation");
            schema_outcome(text)
        }
        "query" => {
            assert!(!typed, "only ad hoc cases have a type expectation");
            let extra = fields.remove("extra").unwrap_or_default();
            Outcome {
                phase: "query",
                errors: query(&extra, text).err().map(|error| vec![error]),
                ty: None,
            }
        }
        "adhoc" => {
            if let Some(extra) = fields.remove("extra") {
                assert!(!typed, "extended-schema cases have no type expectation");
                return Outcome {
                    phase: "adhoc",
                    errors: adhoc_over(&extra, text).err(),
                    ty: None,
                };
            }
            let descriptors = fields.remove("params").unwrap_or_default();
            let params: Vec<_> = descriptors
                .lines()
                .map(|line| {
                    line.split_once('\t')
                        .expect("a parameter has a name and type")
                })
                .collect();
            match adhoc(&params, text) {
                Ok((s, a)) => Outcome {
                    phase: "adhoc",
                    errors: None,
                    ty: typed.then(|| {
                        assert_eq!(a.items.len(), 1, "a typed case has one result");
                        s.show_ty(a.terms.ty(a.items[0]))
                    }),
                },
                Err(errors) => Outcome {
                    phase: "adhoc",
                    errors: Some(errors),
                    ty: None,
                },
            }
        }
        _ => unreachable!("fixture modes were validated"),
    }
}

fn execute(mut case: Case) {
    let text = case.fields.remove("text").expect("a case has text");
    let expected = case
        .fields
        .remove("expect")
        .expect("a case has an expectation");
    let ty = case.fields.remove("type");
    let result = outcome(&case.mode, &text, &mut case.fields, ty.is_some());
    match expected.as_str() {
        "accept" => assert!(
            result.errors.is_none(),
            "accepted case refused in {}: {:?}",
            result.phase,
            result.errors
        ),
        "refuse" => {
            assert!(ty.is_none(), "a refusal has no inferred type");
            let phase = case
                .fields
                .remove("phase")
                .expect("a refusal declares its phase");
            let errors = case
                .fields
                .remove("errors")
                .expect("a refusal declares exact diagnostics");
            let mut expected: Vec<_> = errors
                .split("\n::: diagnostic\n")
                .map(str::to_string)
                .collect();
            assert!(
                expected.iter().all(|e| !e.is_empty()),
                "a refusal has nonempty expected diagnostics"
            );
            let mut actual = result.errors.expect("refused case accepted");
            expected.sort();
            actual.sort();
            assert_eq!(
                result.phase, phase,
                "refusal phase; actual diagnostics: {actual:?}"
            );
            assert_eq!(actual, expected, "exact diagnostic set");
        }
        _ => panic!("unknown expectation {expected}"),
    }
    assert_eq!(result.ty, ty, "exact inferred type");
    assert!(
        case.fields.is_empty(),
        "unknown or unused fields: {:?}",
        case.fields.keys().collect::<Vec<_>>()
    );
}

fn run(mode: &str) {
    let (cases, mut failures) = load();
    let mut declared = BTreeSet::new();
    for case in &cases {
        if !declared.insert(case.name.clone()) {
            failures.push(format!("duplicate case {}", case.name));
        }
    }
    let manifest = std::fs::read_to_string(directory().join("original-cases.txt"))
        .expect("the original-case manifest is readable");
    let fingerprint = manifest
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    assert_eq!(
        fingerprint, 0xd740_58f7_0ff3_0a5b,
        "the original subcase manifest is unchanged"
    );
    let required: BTreeSet<_> = manifest.lines().map(str::to_string).collect();
    assert_eq!(
        required.len(),
        manifest.lines().count(),
        "the preservation manifest contains no duplicate IDs"
    );
    assert!(
        !required.is_empty(),
        "the preservation manifest is nonempty"
    );
    failures.extend(
        required
            .difference(&declared)
            .map(|name| format!("missing original subcase {name}")),
    );
    let selected = cases.iter().filter(|c| c.mode == mode).count();
    assert!(selected > 0, "each driver mode executes cases");
    let mut executed = BTreeSet::new();
    for case in cases.into_iter().filter(|c| c.mode == mode) {
        let name = case.name.clone();
        executed.insert(name.clone());
        if let Err(e) = std::panic::catch_unwind(|| execute(case)) {
            failures.push(format!("{name}: {}", panic_message(e.as_ref())));
        }
    }
    assert_eq!(
        executed.len(),
        selected,
        "every selected subcase executes once"
    );
    assert!(
        failures.is_empty(),
        "checker {mode}: {} declared, {selected} executed, {} failures\n{}",
        declared.len(),
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn schema_cases() {
    run("schema");
}

#[test]
fn adhoc_cases() {
    run("adhoc");
}

#[test]
fn query_cases() {
    run("query");
}
