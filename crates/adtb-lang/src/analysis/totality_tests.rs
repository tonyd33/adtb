use std::collections::{BTreeSet, HashMap};

use super::Totality;
use super::tests::{adhoc, adhoc_normal, reachable};
use crate::check::check_schema;
use crate::kernel::{Build, FileFormat, Kind, Prim};
use crate::norm;

const SPEC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/spec/");
const CASES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/totality/forms.txt");

/// Define [`PRIMS`], each value listed, and [`prim_form`], whose match is exhaustive over the
/// patterns listed, so a new operator compiles only once it is listed.
macro_rules! every_prim {
    ($($value:expr => $pattern:pat),* $(,)?) => {
        /// One value of each operator form, in the order [`prim_form`] numbers them.
        const PRIMS: &[Prim] = &[$($value),*];

        /// Returns the position in [`PRIMS`] of the form of `p`.
        fn prim_form(p: Prim) -> usize {
            let patterns = [$(stringify!($pattern)),*];
            let pattern = match p {
                $($pattern => stringify!($pattern)),*
            };
            patterns
                .iter()
                .position(|q| *q == pattern)
                .expect("every pattern is listed")
        }
    };
}

every_prim! {
    Prim::Add => Prim::Add,
    Prim::Sub => Prim::Sub,
    Prim::Mul => Prim::Mul,
    Prim::Div => Prim::Div,
    Prim::Rem => Prim::Rem,
    Prim::Neg => Prim::Neg,
    Prim::Concat => Prim::Concat,
    Prim::Eq => Prim::Eq,
    Prim::Ne => Prim::Ne,
    Prim::Lt => Prim::Lt,
    Prim::Le => Prim::Le,
    Prim::Gt => Prim::Gt,
    Prim::Ge => Prim::Ge,
    Prim::CheckedAdd => Prim::CheckedAdd,
    Prim::CheckedSub => Prim::CheckedSub,
    Prim::CheckedMul => Prim::CheckedMul,
    Prim::CheckedDiv => Prim::CheckedDiv,
    Prim::CheckedRem => Prim::CheckedRem,
    Prim::CheckedNeg => Prim::CheckedNeg,
    Prim::DecimalDiv => Prim::DecimalDiv,
    Prim::Round => Prim::Round,
    Prim::Rescale => Prim::Rescale,
    Prim::Abs => Prim::Abs,
    Prim::Convert { checked: false } => Prim::Convert { checked: false },
    Prim::Convert { checked: true } => Prim::Convert { checked: true },
    Prim::Wrap => Prim::Wrap,
    Prim::Unwrap => Prim::Unwrap,
    Prim::KeyValue => Prim::KeyValue,
    Prim::ParseUuid => Prim::ParseUuid,
    Prim::ToText => Prim::ToText,
    Prim::BytesLen => Prim::BytesLen,
    Prim::ScalarLen => Prim::ScalarLen,
    Prim::GraphemeLen => Prim::GraphemeLen,
    Prim::StartsWith => Prim::StartsWith,
    Prim::EndsWith => Prim::EndsWith,
    Prim::Contains => Prim::Contains,
    Prim::Matches => Prim::Matches,
    Prim::Lower => Prim::Lower,
    Prim::Upper => Prim::Upper,
    Prim::Fold => Prim::Fold,
    Prim::Collate => Prim::Collate,
    Prim::Utf8 => Prim::Utf8,
    Prim::Bytes => Prim::Bytes,
    Prim::Micros => Prim::Micros,
    Prim::Millis => Prim::Millis,
    Prim::Seconds => Prim::Seconds,
    Prim::Minutes => Prim::Minutes,
    Prim::Hours => Prim::Hours,
    Prim::Days => Prim::Days,
    Prim::Weeks => Prim::Weeks,
    Prim::Months => Prim::Months,
    Prim::Years => Prim::Years,
    Prim::AddPeriod => Prim::AddPeriod,
    Prim::Zoned => Prim::Zoned,
    Prim::InZone => Prim::InZone,
    Prim::ToInstant => Prim::ToInstant,
    Prim::Span => Prim::Span,
    Prim::Start => Prim::Start,
    Prim::End => Prim::End,
    Prim::Overlaps => Prim::Overlaps,
    Prim::Covers => Prim::Covers,
    Prim::Parse => Prim::Parse,
    Prim::Print => Prim::Print,
    Prim::ReadFile { format: FileFormat::Csv, pins: None } => Prim::ReadFile { .. },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Total,
    /// Partial, and total under `try`.
    Fault,
    /// Partial under `try` too.
    Escape,
}

#[derive(Debug, Default)]
struct Case {
    /// The key of the prelude row it covers.
    row: Option<String>,
    /// The spec file and phrase naming the form it covers.
    form: Option<(String, String)>,
    class: Option<Class>,
    params: Vec<(String, String)>,
    schema: String,
    hiding: bool,
    untried: bool,
    /// Each text, with the operator it must reach, by its `Debug` form.
    texts: Vec<(Option<String>, String)>,
    line: usize,
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn squeeze(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cases() -> Vec<Case> {
    let mut out: Vec<Case> = Vec::new();
    for (i, line) in read(CASES).lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if let Some(text) = line.strip_prefix("  ") {
            let case = out.last_mut().expect("a text after a case");
            let (op, text) = match text.split_once(" ~ ") {
                Some((op, text)) => (Some(op.to_string()), text),
                None => (None, text),
            };
            case.texts.push((op, text.to_string()));
            continue;
        }
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        if matches!(word, "row" | "form") {
            out.push(Case {
                line: i + 1,
                ..Case::default()
            });
        }
        let case = out.last_mut().expect("a directive after a case");
        match word {
            "row" => case.row = Some(rest.to_string()),
            "form" => {
                let (file, phrase) = rest.split_once(' ').expect("a file and a phrase");
                let phrase = phrase.trim_matches('"');
                case.form = Some((file.to_string(), phrase.to_string()));
            }
            "expect" => {
                case.class = Some(match rest {
                    "total" => Class::Total,
                    "fault" => Class::Fault,
                    "escape" => Class::Escape,
                    other => panic!("line {}: no class {other:?}", i + 1),
                });
            }
            "given" => {
                case.params = rest
                    .split(", ")
                    .map(|p| {
                        let (n, t) = p.split_once(": ").expect("NAME: TYPE");
                        (n.to_string(), t.to_string())
                    })
                    .collect();
            }
            "schema" => case.schema = format!("{}{rest}\n", case.schema),
            "hiding" => case.hiding = true,
            "untried" => case.untried = true,
            other => panic!("line {}: no directive {other:?}", i + 1),
        }
    }
    out
}

/// Returns each row of a prelude table with a Faults column, by its key, and whether its Faults
/// cell names a fault.
fn prelude_rows() -> HashMap<String, bool> {
    let text = read(&format!("{SPEC}prelude.md"));
    let lines: Vec<&str> = text.lines().collect();
    let mut rows: Vec<(String, String, bool)> = Vec::new();
    let cells = |l: &str| -> Vec<String> {
        l.trim()
            .trim_matches('|')
            .split(" | ")
            .map(|c| c.trim().to_string())
            .collect()
    };
    let mut i = 0;
    while i + 1 < lines.len() {
        let header = cells(lines[i]);
        let faults = header.iter().position(|c| c == "Faults");
        if !(lines[i].starts_with('|') && lines[i + 1].starts_with("| ---")) || faults.is_none() {
            i += 1;
            continue;
        }
        let faults = faults.expect("a Faults column");
        i += 2;
        while i < lines.len() && lines[i].starts_with('|') {
            let row = cells(lines[i]);
            assert_eq!(
                header.len(),
                row.len(),
                "prelude.md:{}: a cell per column",
                i + 1
            );
            let partial = !row[faults].starts_with("none");
            rows.push((row[0].clone(), row[1].clone(), partial));
            i += 1;
        }
    }
    let mut out = HashMap::new();
    for (first, second, partial) in &rows {
        let unique = rows.iter().filter(|(f, _, _)| f == first).count() == 1;
        let key = if unique {
            first.clone()
        } else {
            format!("{first} | {second}")
        };
        assert!(
            out.insert(key.clone(), *partial).is_none(),
            "two rows {key}"
        );
    }
    out
}

/// Returns the text of the section of spec `file` under `heading`, up to its rulings.
fn section(file: &str, heading: &str) -> String {
    let text = read(&format!("{SPEC}{file}"));
    let start = text
        .find(&format!("\n{heading}\n"))
        .unwrap_or_else(|| panic!("{file} has no section {heading}"));
    let rest = &text[start..];
    rest[..rest.find("\nRulings:").unwrap_or(rest.len())].to_string()
}

/// Returns the bullets of the semantics' list of the forms that make an expression partial.
fn partial_forms() -> Vec<String> {
    section("semantics.md", "### 3.4 Totality")
        .split("\n\n")
        .find(|p| p.starts_with("- "))
        .expect("semantics.md lists the partial forms")
        .split("\n- ")
        .map(|b| squeeze(b.trim_start_matches("- ")))
        .collect()
}

/// Returns whether ad hoc `text` is total, and the operators its normal form reaches.
fn judge(case: &Case, text: &str) -> Result<(bool, Vec<Prim>), String> {
    let params: Vec<(&str, &str)> = case
        .params
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    let (s, a) = adhoc(&case.schema, &params, text).map_err(|e| format!("refused: {e:?}"))?;
    let p = check_schema(&s).expect("the schema checks");
    let n = norm::adhoc(&s, &p, &a).map_err(|e| format!("not normalised: {e:?}"))?;
    let item = n.items[0];
    let prims = reachable(&n.terms, item)
        .into_iter()
        .filter_map(|t| match n.terms.kind(t) {
            Kind::Prim(p, _) => Some(*p),
            _ => None,
        })
        .collect();
    let normal = norm::program(&s, &p).map_err(|e| format!("schema not normalised: {e:?}"))?;
    let mut totality =
        Totality::new(&s, case.hiding).with_aggregates(&normal.terms, normal.aggregate_parts());
    Ok((totality.total(&n.terms, item), prims))
}

#[test]
fn every_operator_and_partial_form_the_spec_lists_is_judged_as_listed_alone_and_under_try() {
    let rows = prelude_rows();
    let forms = partial_forms();
    let files: HashMap<&str, String> = [
        ("prelude.md", read(&format!("{SPEC}prelude.md"))),
        ("interop.md", read(&format!("{SPEC}interop.md"))),
        (
            "typing.md",
            section("typing.md", "### 10.1 Total and partial expressions"),
        ),
        ("semantics.md", section("semantics.md", "### 3.4 Totality")),
    ]
    .into_iter()
    .map(|(f, text)| (f, squeeze(&text)))
    .collect();
    let cases = cases();
    let mut wrong: Vec<String> = Vec::new();
    let mut rows_covered: BTreeSet<&str> = BTreeSet::new();
    let mut forms_covered: BTreeSet<usize> = BTreeSet::new();
    let mut reached: BTreeSet<usize> = BTreeSet::new();
    for case in &cases {
        let at = format!("forms.txt:{}", case.line);
        let class = match (&case.row, &case.form) {
            (Some(row), None) => {
                assert!(
                    case.class.is_none(),
                    "{at}: a row's class is its Faults cell"
                );
                let partial = *rows
                    .get(row)
                    .unwrap_or_else(|| panic!("{at}: prelude.md has no row {row}"));
                rows_covered.insert(row);
                if partial { Class::Fault } else { Class::Total }
            }
            (None, Some((file, phrase))) => {
                let spec = files
                    .get(file.as_str())
                    .unwrap_or_else(|| panic!("{at}: no spec file {file}"));
                assert!(
                    spec.contains(phrase.as_str()),
                    "{at}: {file} does not say {phrase:?}"
                );
                if file == "semantics.md" {
                    forms_covered.extend(forms.iter().position(|f| f.contains(phrase.as_str())));
                }
                case.class
                    .unwrap_or_else(|| panic!("{at}: a form states its class"))
            }
            _ => panic!("{at}: a case is a row or a form"),
        };
        assert!(!case.texts.is_empty(), "{at}: a case has a text");
        for (op, text) in &case.texts {
            if case.row.is_some() && op.is_none() {
                wrong.push(format!("{at}: `{text}` names no operator its row names"));
            }
            match judge(case, text) {
                Ok((total, prims)) => {
                    let shown: Vec<String> = prims.iter().map(|p| format!("{p:?}")).collect();
                    if let Some(op) = op.as_ref().filter(|op| !shown.contains(op)) {
                        wrong.push(format!("{at}: `{text}` reaches {shown:?}, not {op}"));
                    }
                    reached.extend(prims.into_iter().map(prim_form));
                    if total != (class == Class::Total) {
                        wrong.push(format!(
                            "{at}: `{text}` judged total {total}, {class:?} as listed"
                        ));
                    }
                }
                Err(e) => wrong.push(format!("{at}: `{text}` {e}")),
            }
            if class == Class::Total || case.untried {
                continue;
            }
            match judge(case, &format!("try ({text})")) {
                Ok((total, _)) if total != (class == Class::Fault) => wrong.push(format!(
                    "{at}: `try ({text})` judged total {total}, {class:?} as listed"
                )),
                Ok(_) => {}
                Err(e) => wrong.push(format!("{at}: `try ({text})` {e}")),
            }
        }
    }
    let missing: Vec<&String> = rows
        .keys()
        .filter(|r| !rows_covered.contains(r.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "prelude.md rows no case covers: {missing:?}"
    );
    let unlisted: Vec<&String> = forms
        .iter()
        .enumerate()
        .filter(|(i, _)| !forms_covered.contains(i))
        .map(|(_, f)| f)
        .collect();
    assert!(
        unlisted.is_empty(),
        "semantics.md forms no case covers: {unlisted:?}"
    );
    for (i, p) in PRIMS.iter().enumerate() {
        assert_eq!(
            i,
            prim_form(*p),
            "`{p:?}` is listed at the position of its form"
        );
    }
    let unreached: Vec<&Prim> = PRIMS
        .iter()
        .filter(|p| !reached.contains(&prim_form(**p)))
        .collect();
    assert!(
        unreached.is_empty(),
        "operators no case reaches: {unreached:?}"
    );
    assert!(
        wrong.is_empty(),
        "judged otherwise than listed:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn an_unreachable_is_partial_and_a_case_is_judged_on_the_arms_it_reaches() {
    let (s, mut n) = adhoc_normal(
        "",
        &[("x", "Shape")],
        "match x | Circle c -> c.r | Square q -> q.side",
    );
    let item = n.items[0];
    let Kind::Case { scrutinee, arms } = n.terms.kind(item).clone() else {
        panic!("a match of a parameter normalises to a `Case`");
    };
    let ty = n.terms.ty(item).clone();
    let mut b = Build::new(&s, &mut n.terms);
    let never = b.unreachable(ty.clone());
    let narrowed = b.case(scrutinee, vec![arms[0], (arms[1].0, arms[1].1, never)], ty);
    let mut totality = Totality::new(&s, false);
    assert!(!totality.total(&n.terms, never));
    assert!(totality.total(&n.terms, narrowed));
}
