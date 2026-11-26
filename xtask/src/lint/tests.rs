use super::*;

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/lint/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("the fixture {} is read: {e}", path.display()))
}

fn rule(query: &str, rows: &[(&str, Kind)]) -> Rule {
    Rule {
        query: query.to_owned(),
        include: Vec::new(),
        exclude: Vec::new(),
        rows: rows
            .iter()
            .map(|&(row, kind)| {
                let declared = Row {
                    kind,
                    why: format!("{row} harms"),
                    fix: format!("{row} fixed"),
                };
                (row.to_owned(), declared)
            })
            .collect(),
    }
}

fn lines(problems: &[Problem]) -> Vec<&str> {
    problems.iter().map(|p| p.line.as_str()).collect()
}

fn sorted_lines(problems: &[Problem]) -> Vec<String> {
    let mut lines: Vec<String> = problems.iter().map(|p| p.line.clone()).collect();
    lines.sort();
    lines
}

fn fixture_text(name: &str) -> String {
    String::from_utf8(fixture(name)).expect("the fixture is UTF-8")
}

fn counts(entries: &[(&str, &str, u64)]) -> Counts {
    let mut counts = Counts::new();
    for &(row, file, n) in entries {
        counts
            .entry(row.to_owned())
            .or_default()
            .insert(file.to_owned(), n);
    }
    counts
}

fn applied(rule: &Rule, known: &[&str], name: &str) -> Result<Tally, String> {
    let mut tally = Tally::default();
    apply(
        rule,
        &known.iter().copied().collect(),
        &fixture(name),
        &mut tally,
    )
    .map(|()| tally)
}

#[test]
fn named_sites_are_findings_of_zero_rows_and_counts_of_ratchet_rows() {
    let house = rule(
        "house.tql",
        &[("em-dash", Kind::Zero), ("doc-verb", Kind::Ratchet)],
    );
    let tally = applied(&house, &["em-dash", "doc-verb", "bold"], "house.json");
    assert_eq!(
        tally,
        Ok(Tally {
            findings: vec![Problem::of(
                "em-dash",
                "crates/a/src/lib.rs:4: em-dash".to_owned()
            )],
            counts: counts(&[("doc-verb", "crates/a/src/lib.rs", 2)]),
        })
    );
}

#[test]
fn named_counts_feed_only_the_rows_of_their_rule() {
    let decode = rule("structure.tql", &[("decode_panics", Kind::Ratchet)]);
    let tally = applied(
        &decode,
        &["pub_fields", "crate_fields", "decode_panics"],
        "structure.json",
    );
    assert_eq!(
        tally,
        Ok(Tally {
            findings: Vec::new(),
            counts: counts(&[
                ("decode_panics", "crates/a/src/b.rs", 3),
                ("decode_panics", "crates/a/src/lib.rs", 17),
            ]),
        })
    );
}

#[test]
fn an_unnamed_count_belongs_to_the_one_row_of_its_rule() {
    let engine = rule("engine.tql", &[("engine_names", Kind::Ratchet)]);
    let tally = applied(&engine, &["engine_names"], "engine.json");
    assert_eq!(
        tally,
        Ok(Tally {
            findings: Vec::new(),
            counts: counts(&[
                ("engine_names", "crates/a/src/b.rs", 0),
                ("engine_names", "crates/a/src/lib.rs", 5),
            ]),
        })
    );
}

#[test]
fn unnamed_ranges_are_findings_on_their_first_line() {
    let prelude = rule("prelude.tql", &[("prelude_positions", Kind::Zero)]);
    let tally = applied(&prelude, &["prelude_positions"], "prelude.json");
    assert_eq!(
        tally,
        Ok(Tally {
            findings: [12, 21]
                .map(|line| Problem::of(
                    "prelude_positions",
                    format!("crates/a/src/lib.rs:{line}: prelude_positions")
                ))
                .into(),
            counts: Counts::new(),
        })
    );
}

#[test]
fn a_zero_row_count_above_zero_is_a_finding() {
    let engine = rule("engine.tql", &[("engine_names", Kind::Zero)]);
    let tally = applied(&engine, &["engine_names"], "engine.json");
    assert_eq!(
        tally,
        Ok(Tally {
            findings: vec![Problem::of(
                "engine_names",
                "crates/a/src/lib.rs: engine_names: 5 sites".to_owned()
            )],
            counts: Counts::new(),
        })
    );
}

#[test]
fn an_undeclared_name_is_refused() {
    let house = rule(
        "house.tql",
        &[("em-dash", Kind::Zero), ("doc-verb", Kind::Ratchet)],
    );
    assert_eq!(
        applied(&house, &["em-dash", "doc-verb"], "house.json"),
        Err("crates/a/src/lib.rs: the row bold is not declared".to_owned())
    );
}

#[test]
fn an_unnamed_value_of_a_rule_with_several_rows_is_refused() {
    let engine = rule("engine.tql", &[("a", Kind::Ratchet), ("b", Kind::Ratchet)]);
    assert_eq!(
        applied(&engine, &["a", "b"], "engine.json"),
        Err("crates/a/src/lib.rs: an unnamed value of a rule with several rows".to_owned())
    );
}

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits in the repository")
}

fn fixture_paths(names: &[&str]) -> BTreeSet<String> {
    names
        .iter()
        .map(|name| format!("xtask/src/lint/fixtures/{name}"))
        .collect()
}

#[test]
fn a_file_that_does_not_parse_is_refused_by_name() {
    let engine = rule("engine.tql", &[("engine_names", Kind::Ratchet)]);
    assert_eq!(
        applied(&engine, &["engine_names"], "unparsed.json"),
        Err(
            "tql cannot parse 1 files:\nxtask/src/lint/fixtures/unparsed.rs.txt:1: does not parse"
                .to_owned()
        )
    );
}

#[test]
fn tql_output_for_a_file_that_does_not_parse_names_it() {
    let engine = rule("engine.tql", &[("engine_names", Kind::Ratchet)]);
    let files = fixture_paths(&["unparsed.rs.txt", "em_dash.rs.txt"]);
    let output = query(repository(), "engine.tql", &files, 1).expect("tql exits 0 or 2");
    let mut tally = Tally::default();
    assert_eq!(
        apply(
            &engine,
            &BTreeSet::from(["engine_names"]),
            &output,
            &mut tally
        ),
        Err(
            "tql cannot parse 1 files:\nxtask/src/lint/fixtures/unparsed.rs.txt:1: does not parse"
                .to_owned()
        )
    );
}

#[test]
fn an_em_dash_in_a_comment_or_string_is_a_finding() {
    let house = rule(
        "house.tql",
        &[
            ("em-dash", Kind::Zero),
            ("bold", Kind::Zero),
            ("section-sign", Kind::Zero),
            ("rule-id", Kind::Zero),
            ("doc-verb", Kind::Zero),
            ("expect-reason", Kind::Zero),
        ],
    );
    let files = fixture_paths(&["em_dash.rs.txt"]);
    let output = query(repository(), "house.tql", &files, 1).expect("tql exits 0");
    let mut tally = Tally::default();
    apply(&house, &BTreeSet::new(), &output, &mut tally).expect("the fixture parses");
    let file = "xtask/src/lint/fixtures/em_dash.rs.txt";
    assert_eq!(
        sorted_lines(&tally.findings),
        [1, 2, 3, 5, 6].map(|line| format!("{file}:{line}: em-dash"))
    );
}

#[test]
fn every_weak_refusal_form_is_a_finding_and_a_bound_or_exact_one_is_not() {
    let refusal = rule("refusal.tql", &[("weak_refusals", Kind::Zero)]);
    let files = fixture_paths(&["refusal.rs.txt"]);
    let output = query(repository(), "refusal.tql", &files, 1).expect("tql exits 0");
    let mut tally = Tally::default();
    apply(&refusal, &BTreeSet::new(), &output, &mut tally).expect("the fixture parses");
    let file = "xtask/src/lint/fixtures/refusal.rs.txt";
    let mut lines: Vec<u64> = (2..=18).collect();
    lines.extend([22, 26, 28]);
    let mut expected: Vec<String> = lines
        .iter()
        .map(|line| format!("{file}:{line}: weak_refusals"))
        .collect();
    expected.sort();
    assert_eq!(sorted_lines(&tally.findings), expected);
}

/// Returns each site the capability and serializer rules find in fixture `name`: each finding of
/// a zero row, and `FILE: ROW: N` for each ratchet row the file counts above zero.
fn capability_sites(name: &str) -> Vec<String> {
    let rules = [
        rule(
            "capabilities.tql",
            &[
                ("abandons_impls", Kind::Ratchet),
                ("unchecked_impls", Kind::Ratchet),
                ("stops_early_judgements", Kind::Ratchet),
                ("admitted_leaves", Kind::Ratchet),
                ("system_leaves", Kind::Ratchet),
                ("owner_authorities", Kind::Ratchet),
            ],
        ),
        rule(
            "capability-names.tql",
            &[("capability_renames", Kind::Zero)],
        ),
        rule(
            "serializer.tql",
            &[
                ("serializer_writes", Kind::Zero),
                ("changed_conflicts", Kind::Zero),
            ],
        ),
    ];
    let files = fixture_paths(&[name]);
    let mut tally = Tally::default();
    for r in &rules {
        let output = query(repository(), &r.query, &files, 1).expect("tql exits 0");
        apply(r, &BTreeSet::new(), &output, &mut tally).expect("the fixture parses");
    }
    let mut sites = sorted_lines(&tally.findings);
    for (row, files) in tally.counts {
        for (file, n) in files {
            if n > 0 {
                sites.push(format!("{file}: {row}: {n}"));
            }
        }
    }
    sites.sort();
    sites
}

#[test]
fn every_renamed_aliased_or_macro_made_capability_and_every_serializer_write_is_a_site() {
    let file = |name: &str| format!("xtask/src/lint/fixtures/{name}");
    for (name, expected) in [
        ("cap_aliased_impl.rs.txt", vec!["1: capability_renames"]),
        (
            "cap_reexport.rs.txt",
            vec!["1: capability_renames", "1: capability_renames"],
        ),
        ("cap_macro_impl.rs.txt", vec!["7: capability_renames"]),
        (
            "cap_judged.rs.txt",
            vec![
                "1: capability_renames",
                "3: capability_renames",
                " stops_early_judgements: 1",
            ],
        ),
        (
            "cap_serializer.rs.txt",
            vec![" serializer_writes: 2 sites", " changed_conflicts: 1 sites"],
        ),
    ] {
        let mut expected: Vec<String> = expected
            .into_iter()
            .map(|site| format!("{}:{site}", file(name)))
            .collect();
        expected.sort();
        assert_eq!(capability_sites(name), expected, "{name}");
    }
}

#[test]
fn the_files_of_a_rule_match_an_include_and_no_exclude() {
    let root = std::env::temp_dir().join(format!("xtask-lint-files-{}", std::process::id()));
    for file in [
        "crates/a/src/lib.rs",
        "crates/a/src/deep/x.rs",
        "crates/a/src/x_tests.rs",
        "crates/a/src/notes.md",
        "crates/a/target/debug/out.rs",
        "crates/a/src/.hidden/h.rs",
        "crates/b/tests/t.rs",
    ] {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().expect("a file under a directory"))
            .expect("the scratch tree is written");
        std::fs::write(&path, "").expect("the scratch tree is written");
    }
    let strings = |patterns: &[&str]| patterns.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
    let found = glob::files(
        &root,
        &strings(&["crates/*/src/**/*.rs", "crates/b/tests/**"]),
        &strings(&["**/*_tests.rs"]),
    );
    let missing = glob::files(&root, &strings(&["crates/*/benches/**"]), &[]);
    std::fs::remove_dir_all(&root).expect("the scratch tree is removed");
    assert_eq!(
        found,
        Ok(BTreeSet::from(
            [
                "crates/a/src/deep/x.rs",
                "crates/a/src/lib.rs",
                "crates/b/tests/t.rs"
            ]
            .map(String::from)
        ))
    );
    assert_eq!(
        missing,
        Err("the pattern \"crates/*/benches/**\" matches no file".to_owned())
    );
}

#[test]
fn the_base_allowlist_is_the_one_at_the_revision_and_empty_without_one() {
    let root = std::env::temp_dir().join(format!("xtask-lint-base-{}", std::process::id()));
    std::fs::create_dir_all(root.join("lint")).expect("the scratch repository is written");
    let jj = |args: &[&str]| {
        let status = Command::new("jj")
            .arg("-R")
            .arg(&root)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("jj starts");
        assert!(status.success(), "jj {args:?}: {status}");
    };
    let init = Command::new("jj")
        .args(["git", "init"])
        .arg(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("jj starts");
    assert!(init.success(), "jj git init: {init}");
    jj(&["describe", "-m", "empty"]);
    jj(&["new"]);
    std::fs::write(root.join(allowlist::PATH), "[r]\n\"a.rs\" = 2\n")
        .expect("the allowlist is written");
    jj(&["describe", "-m", "listed"]);
    jj(&["new"]);
    let listed = base(&root, "@-");
    let empty = base(&root, "@--");
    std::fs::remove_dir_all(&root).expect("the scratch repository is removed");
    assert_eq!(listed, Ok(counts(&[("r", "a.rs", 2)])));
    assert_eq!(empty, Ok(Counts::new()));
}

#[test]
fn a_ratchet_count_must_equal_its_entry() {
    let rules = [rule(
        "structure.tql",
        &[
            ("pub_fields", Kind::Ratchet),
            ("crate_fields", Kind::Ratchet),
        ],
    )];
    let found = counts(&[
        ("pub_fields", "same.rs", 3),
        ("pub_fields", "above.rs", 2),
        ("pub_fields", "unlisted.rs", 1),
        ("pub_fields", "below.rs", 1),
        ("pub_fields", "gone.rs", 0),
    ]);
    let listed = counts(&[
        ("pub_fields", "same.rs", 3),
        ("pub_fields", "above.rs", 1),
        ("pub_fields", "below.rs", 4),
        ("pub_fields", "gone.rs", 2),
        ("crate_fields", "vanished.rs", 1),
        ("doc-verb", "x.rs", 1),
    ]);
    assert_eq!(
        lines(&ratchet(&rules, &found, &listed)),
        [
            "lint/allowlist.toml: [doc-verb] is no ratchet row of lint/rules.toml",
            "vanished.rs: crate_fields has 0, allowed 1: stale, delete the entry",
            "above.rs: pub_fields has 2, allowed 1: remove the sites, do not raise the allowlist",
            "below.rs: pub_fields has 1, allowed 4: stale, lower the entry to 1",
            "gone.rs: pub_fields has 0, allowed 2: stale, delete the entry",
            "unlisted.rs: pub_fields has 1, allowed 0: remove the sites, do not raise the allowlist",
        ]
    );
}

#[test]
fn an_entry_may_not_rise_above_the_base() {
    let listed = counts(&[
        ("r", "same.rs", 2),
        ("r", "fell.rs", 1),
        ("r", "rose.rs", 3),
        ("r", "new.rs", 1),
    ]);
    let base = counts(&[
        ("r", "same.rs", 2),
        ("r", "fell.rs", 2),
        ("r", "rose.rs", 2),
    ]);
    assert_eq!(
        lines(&rises(&listed, &base, "main")),
        [
            "new.rs: r allows 1, the allowlist at main allows 0: a rise is a maintainer's act on main",
            "rose.rs: r allows 3, the allowlist at main allows 2: a rise is a maintainer's act on main",
        ]
    );
}

#[test]
fn a_rendered_allowlist_reads_back_without_its_zero_entries() {
    let found = counts(&[
        ("doc-verb", "a b.rs", 2),
        ("doc-verb", "c.rs", 0),
        ("empty", "d.rs", 0),
    ]);
    assert_eq!(
        allowlist::parse(&allowlist::render(&found)),
        Ok(counts(&[("doc-verb", "a b.rs", 2)]))
    );
}

#[test]
fn an_allowlist_entry_at_zero_is_refused() {
    assert_eq!(
        allowlist::parse("[r]\n\"a.rs\" = 1\n\"b.rs\" = 0\n"),
        Err("[r] lists b.rs at 0: delete the entry".to_owned())
    );
}

#[test]
fn globs_match_by_component() {
    for (pattern, path, matched) in [
        ("crates/*/src/**/*.rs", "crates/adtb/src/lib.rs", true),
        ("crates/*/src/**/*.rs", "crates/adtb/src/heap/row.rs", true),
        ("crates/*/src/**/*.rs", "crates/adtb/tests/a.rs", false),
        ("crates/*/src/**/*.rs", "crates/adtb/src/notes.md", false),
        ("**/tests/**", "crates/adtb-sim/tests/storage/main.rs", true),
        ("**/tests/**", "crates/adtb/src/tests.rs", false),
        ("**/*_tests.rs", "crates/adtb/src/exec/scan_tests.rs", true),
        ("**/*_tests.rs", "crates/adtb/src/exec/tests.rs", false),
        ("**/tests.rs", "tests.rs", true),
        (
            "crates/adtb/src/catalog/codec.rs",
            "crates/adtb/src/catalog/codec.rs",
            true,
        ),
        (
            "crates/adtb/src/catalog/codec.rs",
            "crates/adtb/src/catalog/codecs.rs",
            false,
        ),
    ] {
        assert_eq!(glob::matches(pattern, path), matched, "{pattern} on {path}");
    }
}

#[test]
fn a_row_without_a_why_or_a_fix_or_with_an_unknown_key_is_refused_by_its_rule_and_row() {
    assert_eq!(
        parse_rules(&fixture_text("rules_unexplained.toml")).map(|rules| rules.len()),
        Err([
            "lint/rules.toml: rule 0 (house.tql): the row blank_why has no why",
            "lint/rules.toml: rule 0 (house.tql): the row misspelled has the unknown key fixes",
            "lint/rules.toml: rule 0 (house.tql): the row misspelled has no fix",
            "lint/rules.toml: rule 0 (house.tql): the row no_fix has no fix",
            "lint/rules.toml: rule 1 (engine.tql): the row bare is a string, not a table of kind, why and fix",
        ]
        .join("\n"))
    );
}

#[test]
fn a_row_two_rules_explain_differently_is_refused() {
    assert_eq!(
        parse_rules(&fixture_text("rules_differ.toml")).map(|rules| rules.len()),
        Err("lint/rules.toml: rule 0 (engine.tql) and rule 1 (engine-var.tql) declare the row engine_names with a different kind, why or fix".to_owned())
    );
}

#[test]
fn each_failing_row_prints_its_why_and_fix_once_under_its_sites() {
    let rules = parse_rules(&fixture_text("rules_explained.toml")).expect("the fixture parses");
    let problems = [
        Problem::of("rise", "a.rs:3: rise".to_owned()),
        Problem::of("ratchet", "a.rs: ratchet has 2, allowed 1".to_owned()),
        Problem::of("rise", "b.rs:9: rise".to_owned()),
        Problem {
            row: None,
            line: "lint/allowlist.toml: [gone] is no ratchet row of lint/rules.toml".to_owned(),
        },
    ];
    assert_eq!(
        report(&rules, &problems),
        "lint/allowlist.toml: [gone] is no ratchet row of lint/rules.toml
a.rs: ratchet has 2, allowed 1
  why: A ratchet site holds what it should not.
  fix: Write the other form; a renamed form does not count.
a.rs:3: rise
b.rs:9: rise
  why: A rise lets the harm back in.
  fix: Remove the site; hiding it does not count.
"
    );
}

#[test]
fn every_write_lend_form_is_a_site_and_a_read_lend_a_private_method_or_a_checked_setter_is_not() {
    let lends = rule("lends.tql", &[("write_lends", Kind::Zero)]);
    let files = fixture_paths(&["lends.rs.txt"]);
    let output = query(repository(), "lends.tql", &files, 1).expect("tql exits 0");
    let mut tally = Tally::default();
    apply(&lends, &BTreeSet::new(), &output, &mut tally).expect("the fixture parses");
    let file = "xtask/src/lint/fixtures/lends.rs.txt";
    let mut expected: Vec<String> = [3, 4, 5, 6, 7, 8, 21, 24, 25, 26, 27]
        .iter()
        .map(|line| format!("{file}:{line}: write_lends"))
        .collect();
    expected.sort();
    assert_eq!(sorted_lines(&tally.findings), expected);
}

#[test]
fn union_fields_and_the_fields_a_macro_declares_count_once_per_definition() {
    let fields = rule(
        "structure.tql",
        &[
            ("pub_fields", Kind::Ratchet),
            ("crate_fields", Kind::Ratchet),
        ],
    );
    let files = fixture_paths(&["fields.rs.txt"]);
    let output = query(repository(), "structure.tql", &files, 1).expect("tql exits 0");
    let mut tally = Tally::default();
    apply(
        &fields,
        &BTreeSet::from(["decode_panics", "adtb_decode_panics"]),
        &output,
        &mut tally,
    )
    .expect("the fixture parses");
    let file = "xtask/src/lint/fixtures/fields.rs.txt";
    assert_eq!(
        tally.counts,
        counts(&[("pub_fields", file, 6), ("crate_fields", file, 4)])
    );
}

/// Returns `FILE:LINE` of each site `query` finds for `row` in fixture `name`, sorted.
fn sites_of(query_file: &str, row: &str, name: &str) -> Vec<String> {
    let found = rule(query_file, &[(row, Kind::Zero)]);
    let files = fixture_paths(&[name]);
    let output = query(repository(), query_file, &files, 1).expect("tql exits 0");
    let mut tally = Tally::default();
    apply(&found, &BTreeSet::new(), &output, &mut tally).expect("the fixture parses");
    let mut sites: Vec<String> = lines(&tally.findings)
        .into_iter()
        .map(|line| line.trim_end_matches(&format!(": {row}")).to_owned())
        .collect();
    sites.sort();
    sites
}

fn lines_of(name: &str, lines: &[u64]) -> Vec<String> {
    let mut expected: Vec<String> = lines
        .iter()
        .map(|line| format!("xtask/src/lint/fixtures/{name}:{line}"))
        .collect();
    expected.sort();
    expected
}

#[test]
fn each_collection_constructor_called_or_passed_collect_to_vec_vec_and_a_kept_or_default_is_a_site()
{
    assert_eq!(
        sites_of(
            "write-collections.tql",
            "write_collections",
            "collections.rs.txt"
        ),
        lines_of(
            "collections.rs.txt",
            &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 15, 16, 19]
        )
    );
}

#[test]
fn a_raw_id_decode_by_path_qualified_path_value_macro_argument_rename_or_alias_is_a_site() {
    assert_eq!(
        sites_of("from-raw.tql", "from_raw", "fromraw.rs.txt"),
        lines_of("fromraw.rs.txt", &[1, 2, 4, 5, 6, 7, 10, 11])
    );
}

#[test]
fn an_engine_module_spelled_in_a_const_pattern_comparison_or_list_is_a_site() {
    assert_eq!(
        sites_of("engine.tql", "engine_names", "engine.rs.txt"),
        lines_of(
            "engine.rs.txt",
            &[1, 2, 3, 5, 6, 7, 8, 9, 12, 13, 14, 15, 16]
        )
    );
}

#[test]
fn an_open_helper_closure_macro_or_alias_is_a_site_once_and_a_function_marked_test_is_not() {
    assert_eq!(
        sites_of("simtest.tql", "sim_test_opens", "tests/simtest.rs.txt"),
        lines_of("tests/simtest.rs.txt", &[1, 4, 7, 10, 15, 16, 27, 30, 31])
    );
}

#[test]
fn every_loose_refusal_form_is_a_site_and_the_test_file_forms_only_in_a_test_file() {
    assert_eq!(
        sites_of(
            "loose-refusals.tql",
            "loose_refusals",
            "tests/loose_refusals.rs.txt"
        ),
        lines_of(
            "tests/loose_refusals.rs.txt",
            &[
                1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 18, 22, 25, 26, 27, 28, 29, 31
            ]
        )
    );
    assert_eq!(
        sites_of(
            "loose-refusals.tql",
            "loose_refusals",
            "loose_refusals.rs.txt"
        ),
        lines_of(
            "loose_refusals.rs.txt",
            &[3, 4, 5, 6, 9, 10, 11, 12, 13, 22, 25, 26, 27, 28, 29, 31]
        )
    );
}

#[test]
fn a_panic_by_any_path_macro_or_diverging_function_is_a_decode_panic() {
    let decode = rule("structure.tql", &[("decode_panics", Kind::Ratchet)]);
    let files = fixture_paths(&["decode.rs.txt"]);
    let output = query(repository(), "structure.tql", &files, 1).expect("tql exits 0");
    let mut tally = Tally::default();
    apply(
        &decode,
        &BTreeSet::from(["pub_fields", "crate_fields", "adtb_decode_panics"]),
        &output,
        &mut tally,
    )
    .expect("the fixture parses");
    assert_eq!(
        tally.counts,
        counts(&[("decode_panics", "xtask/src/lint/fixtures/decode.rs.txt", 11)])
    );
}

#[test]
fn every_house_row_reads_block_comments_doc_attributes_and_the_other_spellings() {
    let house = rule(
        "house.tql",
        &[
            ("em-dash", Kind::Zero),
            ("bold", Kind::Zero),
            ("section-sign", Kind::Zero),
            ("rule-id", Kind::Zero),
            ("doc-verb", Kind::Zero),
            ("expect-reason", Kind::Zero),
        ],
    );
    let files = fixture_paths(&["house.rs.txt"]);
    let output = query(repository(), "house.tql", &files, 1).expect("tql exits 0");
    let mut tally = Tally::default();
    apply(&house, &BTreeSet::new(), &output, &mut tally).expect("the fixture parses");
    let file = "xtask/src/lint/fixtures/house.rs.txt";
    let mut expected: Vec<String> = [
        (1, "em-dash"),
        (2, "em-dash"),
        (3, "em-dash"),
        (4, "bold"),
        (5, "bold"),
        (6, "section-sign"),
        (7, "rule-id"),
        (8, "doc-verb"),
        (9, "doc-verb"),
        (10, "bold"),
        (11, "expect-reason"),
        (12, "expect-reason"),
        (13, "expect-reason"),
        (14, "em-dash"),
    ]
    .iter()
    .map(|(line, row)| format!("{file}:{line}: {row}"))
    .collect();
    expected.sort();
    assert_eq!(sorted_lines(&tally.findings), expected);
}
