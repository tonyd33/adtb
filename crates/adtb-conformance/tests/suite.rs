use std::path::{Path, PathBuf};

fn part(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance")
        .join(name)
}

fn passes(dir: &Path) {
    assert!(
        !adtb_conformance::scripts(dir).is_empty(),
        "no scripts in {}",
        dir.display()
    );
    let found = adtb_conformance::run_dir(dir);
    assert!(
        found.is_empty(),
        "{} directives differ:\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn model() {
    passes(&part("model"));
}

#[test]
fn queries() {
    passes(&part("queries"));
}

#[test]
fn reference_queries() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference-queries");
    let required = [
        ("a_path_is_a_join", 3),
        ("a_join_compares_a_reference_with_a_key", 2),
        ("projection_merges_equal_values", 2),
        ("an_optional_reference_is_matched", 2),
        ("a_path_to_one_constructor_needs_one_arm", 2),
        ("a_query_built_sum_selects_by_constructor", 2),
        ("a_backlink_counts_per_row", 2),
        ("a_generator_depends_on_an_earlier_one", 1),
        ("arms_returning_rows_join_at_the_sum", 2),
        ("a_constructor_binder_reaches_header_fields", 1),
        ("an_optional_backlink_skips_none", 1),
        ("a_nested_result_keeps_rows_with_nothing_related", 1),
        ("a_nested_set_flattens_by_ranging_over_it", 1),
        ("group_into_partitions_bindings", 1),
        ("an_aggregate_over_a_group_is_over_a_nonempty_range", 1),
        ("an_aggregate_over_a_possibly_empty_range_is_optional", 1),
        ("an_aggregate_over_a_match_ranges_over_its_result", 1),
        ("functions_and_views_are_called", 1),
        ("a_function_takes_a_lambda", 1),
        ("order_by_lists_the_final_result", 2),
        ("top_takes_the_first_by_a_determinate_order", 1),
        ("a_checked_operator_returns_none_where_it_would_fault", 1),
        ("a_user_defined_aggregate_folds_its_inputs", 3),
    ];
    let paths = adtb_conformance::scripts(&dir);
    let mut reached = std::collections::BTreeMap::<String, usize>::new();
    for path in paths {
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let script =
            adtb_conformance::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut name = None;
        let mut queries = 0;
        for (index, line) in text.lines().enumerate() {
            if let Some(case) = line.strip_prefix("# ") {
                name = Some(case);
            }
            if script.directives.iter().any(|d| {
                d.line == index + 1 && matches!(d.kind, adtb_conformance::Kind::Query { .. })
            }) {
                queries += 1;
                if let Some(case) = name {
                    *reached.entry(case.to_owned()).or_default() += 1;
                }
            }
        }
        assert!(queries > 0, "{} has no query", path.display());
    }
    for (name, outcomes) in required {
        assert!(
            reached.get(name).is_some_and(|n| *n >= outcomes),
            "{name}: must run at least {outcomes} original outcomes; got {:?}",
            reached.get(name)
        );
    }
    passes(&dir);
}

#[test]
fn writes() {
    passes(&part("writes"));
}

#[test]
fn constraints() {
    passes(&part("constraints"));
}

#[test]
fn conformance() {
    passes(&part("conformance"));
}

#[test]
fn interop() {
    passes(&part("interop"));
}

fn expand(body: &str, schema: &str, run: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        match line {
            "@SCHEMA@" => out.push_str(schema),
            "@RUN@" => out.push_str(run),
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

#[test]
fn generated_queries_match_their_bodies() {
    let gen_dir = part("gen/queries");
    let read = |path: &Path| {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };
    let schema = read(&gen_dir.join("schema.txt"));
    let run = read(&gen_dir.join("run.txt"));
    let mut bodies: Vec<PathBuf> = std::fs::read_dir(gen_dir.join("bodies"))
        .expect("the bodies directory lists")
        .map(|entry| entry.expect("a bodies entry reads").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "in"))
        .collect();
    bodies.sort();
    assert!(!bodies.is_empty(), "no bodies in {}", gen_dir.display());
    let drifted: Vec<String> = bodies
        .iter()
        .filter_map(|body| {
            let name = body.file_stem().expect("a body has a name");
            let generated = part("queries").join(name);
            (read(&generated) != expand(&read(body), &schema, &run))
                .then(|| generated.display().to_string())
        })
        .collect();
    assert!(
        drifted.is_empty(),
        "generated files differ from their bodies; rerun conformance/gen/queries/build.sh:\n{}",
        drifted.join("\n")
    );
}
