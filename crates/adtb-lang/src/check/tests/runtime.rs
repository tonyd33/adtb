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
            matches!(mode, "schema" | "adhoc" | "query" | "probe"),
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
            let loaded = parse(name, &text);
            assert!(
                loaded.iter().all(|c| c.mode != "probe"),
                "typed probes use .probe files"
            );
            loaded
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

type Constructor = (Vec<String>, u32, String, Vec<String>);

pub(super) fn constructors(terms: &Terms, root: Tm) -> Vec<Constructor> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(t) = stack.pop() {
        if let (Kind::Ctor { ctor, fields }, Ty::Sum { ctors, .. }) = (terms.kind(t), terms.ty(t)) {
            out.push((
                ctors.iter().map(|(c, _)| c.clone()).collect(),
                *ctor,
                ctors[*ctor as usize].0.clone(),
                fields.iter().map(|(f, _)| f.clone()).collect(),
            ));
        }
        stack.extend(terms.children(t));
    }
    out.sort();
    out.dedup();
    out
}

fn assert_constructors(terms: &Terms, root: Tm, text: &str) {
    let expected: Vec<Constructor> = text
        .lines()
        .map(|line| {
            let columns: Vec<_> = line.split('\t').collect();
            assert_eq!(
                columns.len(),
                4,
                "a constructor expectation has four columns"
            );
            (
                columns[0].split('|').map(str::to_string).collect(),
                columns[1].parse().expect("a raw constructor index"),
                columns[2].to_string(),
                columns[3]
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
            )
        })
        .collect();
    assert!(
        !expected.is_empty(),
        "constructor expectations are nonempty"
    );
    assert_eq!(
        constructors(terms, root),
        expected,
        "exact constructor tuples"
    );
}

fn schema_probes(s: &Schema, p: &Program, fields: &mut BTreeMap<String, String>) {
    if let Some(expected) = fields.remove("sexp") {
        let name = fields.remove("body").expect("a printed body has a name");
        assert_eq!(
            sexp(&p.terms, body_of(s, p, &name)),
            expected,
            "exact body sexp"
        );
    }
    if let Some(expected) = fields.remove("constructors") {
        let names = fields
            .remove("bodies")
            .expect("constructor probes name their bodies");
        let root_ctor = fields
            .remove("root_ctor")
            .map(|s| s.parse::<u32>().expect("a raw root constructor index"));
        assert!(!names.is_empty(), "constructor probes have bodies");
        for name in names.lines() {
            let root = body_of(s, p, name);
            if let Some(expected) = root_ctor {
                let Kind::Ctor { ctor, .. } = p.terms.kind(root) else {
                    panic!("a probed body has a constructor root");
                };
                assert_eq!(*ctor, expected, "exact root constructor index");
            }
            assert_constructors(&p.terms, root, &expected);
        }
    }
}

fn schema_outcome(text: &str, fields: &mut BTreeMap<String, String>) -> Outcome {
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
    let errors = match result {
        Ok((s, p)) => {
            schema_probes(&s, &p, fields);
            None
        }
        Err(errors) => Some(errors),
    };
    Outcome {
        phase: "check",
        errors,
        ty: None,
    }
}

fn outcome(mode: &str, text: &str, fields: &mut BTreeMap<String, String>, typed: bool) -> Outcome {
    match mode {
        "schema" => {
            assert!(!typed, "only ad hoc cases have a type expectation");
            schema_outcome(text, fields)
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
                Ok((s, a)) => {
                    if let Some(expected) = fields.remove("constructors") {
                        assert_eq!(a.items.len(), 1, "a constructor probe has one result");
                        assert_constructors(&a.terms, a.items[0], &expected);
                    }
                    Outcome {
                        phase: "adhoc",
                        errors: None,
                        ty: typed.then(|| {
                            assert_eq!(a.items.len(), 1, "a typed case has one result");
                            s.show_ty(a.terms.ty(a.items[0]))
                        }),
                    }
                }
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
    for (file, expected) in [
        ("original-cases.txt", 0xd740_58f7_0ff3_0a5b),
        ("additional-original-cases.txt", 0xcb44a3bfab6e357d),
    ] {
        let manifest = std::fs::read_to_string(directory().join(file))
            .expect("the original-case manifest is readable");
        let fingerprint = manifest
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
            });
        assert_eq!(
            fingerprint, expected,
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
    }
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

const PROPERTIES: &[(&str, fn())] = &[
    ("navigation_desugars_to_deref", navigation_desugars_to_deref),
    (
        "a_backlink_desugars_to_a_filtered_comprehension",
        a_backlink_desugars_to_a_filtered_comprehension,
    ),
    (
        "a_comprehension_is_nested_for_if_singleton",
        a_comprehension_is_nested_for_if_singleton,
    ),
    ("exists_tests_emptiness", exists_tests_emptiness),
    ("and_or_not_are_ifs", and_or_not_are_ifs),
    (
        "constructor_relations_are_relations",
        constructor_relations_are_relations,
    ),
    (
        "a_match_on_a_relation_row_is_a_case_per_constructor",
        a_match_on_a_relation_row_is_a_case_per_constructor,
    ),
    (
        "a_match_on_a_constructor_set_cases_over_the_set",
        a_match_on_a_constructor_set_cases_over_the_set,
    ),
    (
        "an_or_pattern_shares_its_body",
        an_or_pattern_shares_its_body,
    ),
    (
        "a_scalar_match_is_a_chain_without_case",
        a_scalar_match_is_a_chain_without_case,
    ),
    ("a_match_on_a_key_derefs_it", a_match_on_a_key_derefs_it),
    (
        "a_group_desugars_to_group_over_binder_records",
        a_group_desugars_to_group_over_binder_records,
    ),
    (
        "constructor_fields_pair_with_their_written_names",
        constructor_fields_pair_with_their_written_names,
    ),
    (
        "an_aggregate_is_an_aggregate_node",
        an_aggregate_is_an_aggregate_node,
    ),
    ("top_is_a_set_at_any_depth", top_is_a_set_at_any_depth),
    ("fix_is_a_kernel_fix", fix_is_a_kernel_fix),
    (
        "a_function_is_one_lam_per_parameter",
        a_function_is_one_lam_per_parameter,
    ),
    ("a_view_is_a_lam_over_unit", a_view_is_a_lam_over_unit),
    (
        "applying_a_function_value_nests_apps",
        applying_a_function_value_nests_apps,
    ),
    (
        "a_declared_call_keeps_its_arguments",
        a_declared_call_keeps_its_arguments,
    ),
    (
        "a_function_named_without_a_call_is_nested_lams_over_its_call",
        a_function_named_without_a_call_is_nested_lams_over_its_call,
    ),
    (
        "a_lambda_of_several_parameters_is_nested_lams",
        a_lambda_of_several_parameters_is_nested_lams,
    ),
    (
        "text_literals_elaborate_to_nfc",
        text_literals_elaborate_to_nfc,
    ),
    (
        "an_aggregate_over_a_group_has_a_nonempty_source",
        an_aggregate_over_a_group_has_a_nonempty_source,
    ),
    (
        "an_index_field_through_a_value_level_constructor_names_its_path",
        an_index_field_through_a_value_level_constructor_names_its_path,
    ),
];

pub(super) struct Probe {
    case: Case,
    consumed: RefCell<BTreeSet<String>>,
}

impl Probe {
    pub(super) fn text(&self, key: &str) -> &str {
        self.consumed.borrow_mut().insert(key.to_string());
        self.case
            .fields
            .get(key)
            .unwrap_or_else(|| panic!("{}: missing field {key}", self.case.name))
    }

    pub(super) fn number<T: std::str::FromStr>(&self, key: &str) -> T {
        self.text(key)
            .parse()
            .unwrap_or_else(|_| panic!("{}: invalid number in {key}", self.case.name))
    }

    pub(super) fn numbers<T: std::str::FromStr>(&self, key: &str) -> Vec<T> {
        self.text(key)
            .lines()
            .map(|line| {
                line.parse()
                    .unwrap_or_else(|_| panic!("{}: invalid number in {key}", self.case.name))
            })
            .collect()
    }

    pub(super) fn aggregates(&self, key: &str) -> Vec<(crate::kernel::Agg, bool)> {
        use crate::kernel::Agg;
        let values: Vec<_> = self
            .text(key)
            .lines()
            .map(|line| {
                let (op, nonempty) = line
                    .split_once('\t')
                    .expect("an aggregate expectation has an operation and flag");
                let op = match op {
                    "min" => Agg::Min,
                    "max" => Agg::Max,
                    "avg" => Agg::Avg,
                    _ => panic!("unknown expected aggregate {op}"),
                };
                (op, nonempty.parse().expect("a nonempty-source flag"))
            })
            .collect();
        assert!(!values.is_empty(), "an aggregate probe has expectations");
        values
    }

    pub(super) fn paths(&self, key: &str) -> Vec<Vec<PathStep>> {
        let paths: Vec<_> = self
            .text(key)
            .lines()
            .map(|line| {
                if line == "empty" {
                    return Vec::new();
                }
                line.split(',')
                    .map(|step| {
                        let (kind, value) = step
                            .split_once(' ')
                            .expect("an index path step has a kind and value");
                        match kind {
                            "field" => PathStep::Field(value.into()),
                            "ctor" => {
                                PathStep::Ctor(value.parse().expect("a raw path constructor index"))
                            }
                            _ => panic!("unknown index path step {kind}"),
                        }
                    })
                    .collect()
            })
            .collect();
        assert!(!paths.is_empty(), "an index probe has expected paths");
        paths
    }

    pub(super) fn finish(self) {
        let consumed = self.consumed.into_inner();
        let declared: BTreeSet<_> = self.case.fields.into_keys().collect();
        assert_eq!(
            consumed, declared,
            "{}: unknown or unused probe fields",
            self.case.name
        );
    }
}

fn load_probes(name: &str) -> Vec<Probe> {
    let known: BTreeSet<_> = PROPERTIES.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        known.len(),
        PROPERTIES.len(),
        "property entry points have unique names"
    );
    assert!(
        known.contains(name),
        "a probe has a typed assertion entry point"
    );
    for entry in std::fs::read_dir(directory()).expect("the checker fixtures are readable") {
        let path = entry.expect("a probe entry is readable").path();
        if path.extension().is_some_and(|e| e == "probe") {
            let stem = path
                .file_stem()
                .expect("a probe has a stem")
                .to_str()
                .expect("a UTF-8 probe name");
            assert!(known.contains(stem), "unknown probe property {stem}");
        }
    }
    let text = std::fs::read_to_string(directory().join(format!("{name}.probe")))
        .expect("the probe fixture is readable");
    let cases = parse(name, &text);
    let names: BTreeSet<_> = cases.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names.len(), cases.len(), "probe case IDs are unique");
    let manifest = std::fs::read_to_string(directory().join("probe-original-cases.txt"))
        .expect("the probe manifest is readable");
    let fingerprint = manifest
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    assert_eq!(
        fingerprint, 0x82228ba235395dad,
        "the original probe manifest is unchanged"
    );
    let required: BTreeSet<_> = manifest.lines().collect();
    assert_eq!(
        required.len(),
        manifest.lines().count(),
        "probe manifest IDs are unique"
    );
    let prefix = format!("{name}/");
    let originals: Vec<_> = required
        .iter()
        .filter(|id| id.starts_with(&prefix))
        .collect();
    assert!(
        !originals.is_empty(),
        "each property has original probe cases"
    );
    for id in originals {
        assert!(names.contains(id), "missing original probe {id}");
    }
    cases
        .into_iter()
        .map(|case| {
            assert_eq!(case.mode, "probe", "a probe fixture declares probe cases");
            Probe {
                case,
                consumed: RefCell::new(BTreeSet::new()),
            }
        })
        .collect()
}

pub(super) fn probes(name: &str, check: impl Fn(&Probe)) {
    let cases = load_probes(name);
    let selected = cases.len();
    let mut executed = BTreeSet::new();
    let mut failures = Vec::new();
    for case in cases {
        let name = case.case.name.clone();
        assert!(executed.insert(name.clone()), "each probe executes once");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            check(&case);
            case.finish();
        }));
        if let Err(error) = result {
            failures.push(format!("{name}: {}", panic_message(error.as_ref())));
        }
    }
    assert_eq!(
        executed.len(),
        selected,
        "every selected probe executes once"
    );
    assert!(
        failures.is_empty(),
        "probe {name}: {selected} executed, {} failures\n{}",
        failures.len(),
        failures.join("\n")
    );
}
