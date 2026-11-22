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
            .map(|&(row, kind)| (row.to_owned(), kind))
            .collect(),
    }
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
            findings: vec!["crates/a/src/lib.rs:4: em-dash".to_owned()],
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
            findings: vec![
                "crates/a/src/lib.rs:12: prelude_positions".to_owned(),
                "crates/a/src/lib.rs:21: prelude_positions".to_owned(),
            ],
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
            findings: vec!["crates/a/src/lib.rs: engine_names: 5 sites".to_owned()],
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
    tally.findings.sort();
    let file = "xtask/src/lint/fixtures/em_dash.rs.txt";
    assert_eq!(
        tally.findings,
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
    tally.findings.sort();
    assert_eq!(tally.findings, expected);
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
    let mut sites = tally.findings;
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
        ratchet(&rules, &found, &listed),
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
        rises(&listed, &base, "main"),
        [
            "new.rs: r allows 1, the allowlist at main allows 0: a rise is the lead's act on main",
            "rose.rs: r allows 3, the allowlist at main allows 2: a rise is the lead's act on main",
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
