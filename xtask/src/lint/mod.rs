//! `cargo xtask lint`: the TQL rules under `lint/`, as `lint/rules.toml` declares them.

mod allowlist;
mod glob;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

use crate::step;
use allowlist::Counts;

pub const STEP: &str = "lint";

const RULES: &str = "lint/rules.toml";

const TQL_VERSION: &str = "0.4.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Every site is a finding.
    Zero,
    /// A file's sites may not differ from its allowlist entry.
    Ratchet,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rules {
    rule: Vec<Declared>,
}

/// A rule as `lint/rules.toml` writes it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Declared {
    query: String,
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    rows: BTreeMap<String, toml::Value>,
}

/// One run of a query over one file set, and the rows it feeds.
#[derive(Debug)]
struct Rule {
    query: String,
    include: Vec<String>,
    exclude: Vec<String>,
    rows: BTreeMap<String, Row>,
}

/// A row a rule feeds, with the text `cargo xtask lint` prints under its failures.
#[derive(Debug, PartialEq, Eq)]
struct Row {
    kind: Kind,
    /// The harm the row stops.
    why: String,
    /// What to write instead, and the rewrites that do not count as a fix.
    fix: String,
}

/// A line `cargo xtask lint` prints, under the row it is about when it is about one.
#[derive(Debug, PartialEq, Eq)]
struct Problem {
    row: Option<String>,
    line: String,
}

impl Problem {
    fn of(row: &str, line: String) -> Self {
        Self {
            row: Some(row.to_owned()),
            line,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Measure {
    Count(u64),
    /// A site on this 1-based line.
    Site(u64),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Tally {
    /// `FILE:LINE: ROW`, or `FILE: ROW: N sites` for a count, for each site of a zero row.
    findings: Vec<Problem>,
    counts: Counts,
}

/// Run every rule over the tree at `root`, and with `against`, check the allowlist against the
/// one at that revision. With `counts`, print the allowlist for the tree instead.
pub fn run(root: &Path, against: Option<&str>, counts: bool) -> Result<(), String> {
    check_version(root)?;
    let rules = rules(root)?;
    let tally = tally(root, &rules)?;
    if counts {
        return write_stdout(&allowlist::render(&tally.counts));
    }
    let listed = read(root, allowlist::PATH).and_then(|text| {
        allowlist::parse(&text).map_err(|e| format!("{}: {e}", allowlist::PATH))
    })?;
    let mut problems = tally.findings;
    problems.extend(ratchet(&rules, &tally.counts, &listed));
    if let Some(revision) = against {
        problems.extend(rises(&listed, &base(root, revision)?, revision));
    }
    if problems.is_empty() {
        return Ok(());
    }
    write_stdout(&report(&rules, &problems))?;
    Err(format!("{STEP} failed: {} problems", problems.len()))
}

fn rules(root: &Path) -> Result<Vec<Rule>, String> {
    parse_rules(&read(root, RULES)?)
}

/// Returns the rules `text` declares.
///
/// Postconditions: every rule has a row; every row has a kind, a `why` and a `fix` that are not
/// blank, and no other key; a row two rules declare has the same kind, `why` and `fix` in both.
fn parse_rules(text: &str) -> Result<Vec<Rule>, String> {
    let rules: Rules = toml::from_str(text).map_err(|e| format!("{RULES}: {e}"))?;
    let mut refused = Vec::new();
    let mut parsed: Vec<Rule> = Vec::new();
    for (index, declared) in rules.rule.into_iter().enumerate() {
        let name = format!("rule {index} ({})", declared.query);
        if declared.rows.is_empty() {
            return Err(format!("{RULES}: {name} has no rows"));
        }
        let mut rows = BTreeMap::new();
        for (row, value) in &declared.rows {
            match parse_row(value) {
                Ok(parsed_row) => {
                    rows.insert(row.clone(), parsed_row);
                }
                Err(faults) => refused.extend(
                    faults
                        .into_iter()
                        .map(|fault| format!("{RULES}: {name}: the row {row} {fault}")),
                ),
            }
        }
        for (row, declared_row) in &rows {
            let earlier = parsed
                .iter()
                .enumerate()
                .find_map(|(i, rule)| rule.rows.get(row).map(|r| (i, rule, r)));
            if let Some((other, rule, earlier)) = earlier
                && earlier != declared_row
            {
                return Err(format!(
                    "{RULES}: rule {other} ({}) and {name} declare the row {row} with a different kind, why or fix",
                    rule.query
                ));
            }
        }
        parsed.push(Rule {
            query: declared.query,
            include: declared.include,
            exclude: declared.exclude,
            rows,
        });
    }
    if refused.is_empty() {
        Ok(parsed)
    } else {
        Err(refused.join("\n"))
    }
}

/// Returns the row `value` declares: a table of `kind`, `why` and `fix`. Otherwise returns each
/// fault, as the rest of a sentence about the row.
fn parse_row(value: &toml::Value) -> Result<Row, Vec<String>> {
    let Some(table) = value.as_table() else {
        return Err(vec![format!(
            "is a {}, not a table of kind, why and fix",
            value.type_str()
        )]);
    };
    let mut faults: Vec<String> = table
        .keys()
        .filter(|key| !["kind", "why", "fix"].contains(&key.as_str()))
        .map(|key| format!("has the unknown key {key}"))
        .collect();
    let kind = match table.get("kind").and_then(toml::Value::as_str) {
        Some("zero") => Some(Kind::Zero),
        Some("ratchet") => Some(Kind::Ratchet),
        _ => {
            faults.push("has no kind zero or ratchet".to_owned());
            None
        }
    };
    let mut text = |key: &str| match table.get(key).and_then(toml::Value::as_str) {
        Some(text) if !text.trim().is_empty() => Some(text.to_owned()),
        _ => {
            faults.push(format!("has no {key}"));
            None
        }
    };
    let why = text("why");
    let fix = text("fix");
    match (kind, why, fix) {
        (Some(kind), Some(why), Some(fix)) if faults.is_empty() => Ok(Row { kind, why, fix }),
        _ => Err(faults),
    }
}

/// Returns the text `cargo xtask lint` prints for `problems`: the lines about no row, then each
/// row's lines followed once by its `why` and `fix`.
fn report(rules: &[Rule], problems: &[Problem]) -> String {
    let mut by_row: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut text = String::new();
    for problem in problems {
        match &problem.row {
            Some(row) => by_row.entry(row).or_default().push(&problem.line),
            None => text.push_str(&format!("{}\n", problem.line)),
        }
    }
    for (row, lines) in by_row {
        for line in lines {
            text.push_str(&format!("{line}\n"));
        }
        match rules.iter().find_map(|rule| rule.rows.get(row)) {
            Some(declared) => text.push_str(&format!(
                "  why: {}\n  fix: {}\n",
                declared.why, declared.fix
            )),
            None => text.push_str(&format!("  {RULES} declares no row {row}\n")),
        }
    }
    text
}

fn tally(root: &Path, rules: &[Rule]) -> Result<Tally, String> {
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    let mut tally = Tally::default();
    for (index, rule) in rules.iter().enumerate() {
        let name = format!("rule {index} ({})", rule.query);
        for (row, declared) in &rule.rows {
            if declared.kind == Kind::Ratchet {
                tally.counts.entry(row.clone()).or_default();
            }
        }
        let files = glob::files(root, &rule.include, &rule.exclude)
            .map_err(|e| format!("{STEP}: {name}: {e}"))?;
        if files.is_empty() {
            return Err(format!(
                "{STEP}: {name}: its include and exclude patterns leave no file"
            ));
        }
        let output = query(root, &rule.query, &files, workers)
            .map_err(|e| format!("{STEP}: {name}: {e}"))?;
        let known: BTreeSet<&str> = rules
            .iter()
            .filter(|other| other.query == rule.query)
            .flat_map(|other| other.rows.keys().map(String::as_str))
            .collect();
        apply(rule, &known, &output, &mut tally).map_err(|e| format!("{STEP}: {name}: {e}"))?;
    }
    Ok(tally)
}

/// Returns tql's JSON output for `lint/QUERY` over `files`. tql exits 2 when a file does not
/// parse and still prints its JSON, which names each such file.
fn query(
    root: &Path,
    query: &str,
    files: &BTreeSet<String>,
    workers: usize,
) -> Result<Vec<u8>, String> {
    let output = tql(root)
        .args([
            "query",
            "-g",
            "rust",
            "-w",
            &workers.to_string(),
            "--format",
            "json",
            "-f",
        ])
        .arg(Path::new("lint").join(query))
        .args(files)
        .stderr(Stdio::inherit())
        .output()
        .map_err(started)?;
    match output.status.code() {
        Some(0 | 2) => Ok(output.stdout),
        _ => Err(format!(
            "tql over {} files failed: {}",
            files.len(),
            output.status
        )),
    }
}

fn tql(root: &Path) -> Command {
    let mut tql = Command::new("tql");
    tql.current_dir(root);
    tql
}

fn started(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        format!("tql is not on PATH: the dev shell puts tql {TQL_VERSION} there")
    } else {
        format!("tql did not start: {e}")
    }
}

fn check_version(root: &Path) -> Result<(), String> {
    let output = tql(root)
        .arg("version")
        .output()
        .map_err(|e| format!("{STEP}: {}", started(e)))?;
    let printed = String::from_utf8_lossy(&output.stdout);
    if output.status.success() && printed.trim() == format!("tql version {TQL_VERSION}") {
        Ok(())
    } else {
        Err(format!(
            "{STEP}: `tql version` printed {:?} and {}; lint needs tql {TQL_VERSION}",
            printed.trim(),
            output.status
        ))
    }
}

/// Add the values of `output`, tql's JSON, to `tally` under the rows of `rule`. A value named by
/// a row another rule of the same query declares is skipped.
fn apply(
    rule: &Rule,
    known: &BTreeSet<&str>,
    output: &[u8],
    tally: &mut Tally,
) -> Result<(), String> {
    let output: Value =
        serde_json::from_slice(output).map_err(|e| format!("tql printed no JSON: {e}"))?;
    let results = output["results"]
        .as_array()
        .ok_or("tql printed no `results` array")?;
    let mut unparsed = Vec::new();
    for result in results {
        let file = result["file"].as_str().ok_or("a result with no `file`")?;
        let errors = result["syntax_errors"]
            .as_array()
            .ok_or_else(|| format!("{file}: no `syntax_errors` array"))?;
        if let Some(error) = errors.first() {
            let line = error["start_point"]["row"]
                .as_u64()
                .map_or(0, |row| row + 1);
            unparsed.push(format!("{file}:{line}: does not parse"));
            continue;
        }
        let values = result["values"]
            .as_array()
            .ok_or_else(|| format!("{file}: no `values` array"))?;
        for value in values {
            for (name, measure) in decode(value).map_err(|e| format!("{file}: {e}: {value}"))? {
                let row = match name {
                    Some(name) if rule.rows.contains_key(&name) => name,
                    Some(name) if known.contains(name.as_str()) => continue,
                    Some(name) => return Err(format!("{file}: the row {name} is not declared")),
                    None => match rule.rows.keys().collect::<Vec<_>>().as_slice() {
                        [row] => (*row).clone(),
                        _ => {
                            return Err(format!(
                                "{file}: an unnamed value of a rule with several rows"
                            ));
                        }
                    },
                };
                record(rule.rows[&row].kind, &row, file, measure, tally);
            }
        }
    }
    if unparsed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "tql cannot parse {} files:\n{}",
            unparsed.len(),
            unparsed.join("\n")
        ))
    }
}

fn record(kind: Kind, row: &str, file: &str, measure: Measure, tally: &mut Tally) {
    match (kind, measure) {
        (Kind::Zero, Measure::Site(line)) => tally
            .findings
            .push(Problem::of(row, format!("{file}:{line}: {row}"))),
        (Kind::Zero, Measure::Count(0)) => {}
        (Kind::Zero, Measure::Count(n)) => tally
            .findings
            .push(Problem::of(row, format!("{file}: {row}: {n} sites"))),
        (Kind::Ratchet, measure) => {
            let n = match measure {
                Measure::Count(n) => n,
                Measure::Site(_) => 1,
            };
            *tally
                .counts
                .entry(row.to_owned())
                .or_default()
                .entry(file.to_owned())
                .or_default() += n;
        }
    }
}

/// Returns the measures in one of tql's values: a count, a range, `[NAME, VALUE]` naming one,
/// or an array of these.
fn decode(value: &Value) -> Result<Vec<(Option<String>, Measure)>, String> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .map(|n| vec![(None, Measure::Count(n))])
            .ok_or_else(|| "a count that is no natural number".to_owned()),
        Value::Object(_) => value["start_point"]["row"]
            .as_u64()
            .map(|row| vec![(None, Measure::Site(row + 1))])
            .ok_or_else(|| "an object that is no range".to_owned()),
        Value::Array(items) => match items.as_slice() {
            [Value::String(name), inner] => {
                let mut measures = decode(inner)?;
                match (measures.pop(), measures.is_empty()) {
                    (Some((None, measure)), true) => Ok(vec![(Some(name.clone()), measure)]),
                    _ => Err(format!("the value named {name} is no count or range")),
                }
            }
            _ => items.iter().try_fold(Vec::new(), |mut all, item| {
                all.extend(decode(item)?);
                Ok(all)
            }),
        },
        _ => Err("a value that is no count, range or array".to_owned()),
    }
}

/// Returns a line for each file whose count of a ratchet row differs from its allowlist entry,
/// and for each allowlisted row no rule declares a ratchet.
fn ratchet(rules: &[Rule], counts: &Counts, listed: &Counts) -> Vec<Problem> {
    let ratchets: BTreeSet<&str> = rules
        .iter()
        .flat_map(|rule| rule.rows.iter())
        .filter(|(_, row)| row.kind == Kind::Ratchet)
        .map(|(row, _)| row.as_str())
        .collect();
    let mut problems: Vec<Problem> = listed
        .keys()
        .filter(|row| !ratchets.contains(row.as_str()))
        .map(|row| Problem {
            row: None,
            line: format!("{}: [{row}] is no ratchet row of {RULES}", allowlist::PATH),
        })
        .collect();
    let empty = BTreeMap::new();
    for row in ratchets {
        let found = counts.get(row).unwrap_or(&empty);
        let allowed = listed.get(row).unwrap_or(&empty);
        let files: BTreeSet<&String> = found.keys().chain(allowed.keys()).collect();
        for file in files {
            let have = found.get(file).copied().unwrap_or(0);
            let want = allowed.get(file).copied().unwrap_or(0);
            if have > want {
                problems.push(Problem::of(
                    row,
                    format!(
                        "{file}: {row} has {have}, allowed {want}: remove the sites, do not raise the allowlist"
                    ),
                ));
            } else if have < want {
                let fix = if have == 0 {
                    "delete the entry".to_owned()
                } else {
                    format!("lower the entry to {have}")
                };
                problems.push(Problem::of(
                    row,
                    format!("{file}: {row} has {have}, allowed {want}: stale, {fix}"),
                ));
            }
        }
    }
    problems
}

/// Returns a line for each entry of `listed` above the one of `base`, an absent entry counting
/// as 0.
fn rises(listed: &Counts, base: &Counts, revision: &str) -> Vec<Problem> {
    let mut problems = Vec::new();
    for (row, files) in listed {
        for (file, &count) in files {
            let before = base
                .get(row)
                .and_then(|b| b.get(file))
                .copied()
                .unwrap_or(0);
            if count > before {
                problems.push(Problem::of(
                    row,
                    format!(
                        "{file}: {row} allows {count}, the allowlist at {revision} allows {before}: a rise is a maintainer's act on main"
                    ),
                ));
            }
        }
    }
    problems
}

/// Returns the allowlist at `revision`, empty when `revision` has none.
fn base(root: &Path, revision: &str) -> Result<Counts, String> {
    let jj = |subcommand: &str| {
        let mut jj = Command::new("jj");
        jj.arg("-R")
            .arg(root)
            .args([
                "--ignore-working-copy",
                "--color=never",
                "file",
                subcommand,
                "-r",
                revision,
            ])
            .arg(format!("root:{:?}", allowlist::PATH));
        jj
    };
    let listed = step::output(STEP, &mut jj("list"))?;
    if listed.is_empty() {
        step::say(format_args!(
            "{STEP}: {revision} has no {}, so each entry counts as a rise from 0",
            allowlist::PATH
        ))?;
        return Ok(Counts::new());
    }
    let text = String::from_utf8(step::output(STEP, &mut jj("show"))?)
        .map_err(|e| format!("{STEP}: {} at {revision} is no UTF-8: {e}", allowlist::PATH))?;
    allowlist::parse(&text).map_err(|e| format!("{} at {revision}: {e}", allowlist::PATH))
}

fn read(root: &Path, path: &str) -> Result<String, String> {
    std::fs::read_to_string(root.join(path)).map_err(|e| format!("{STEP}: reading {path}: {e}"))
}

fn write_stdout(text: &str) -> Result<(), String> {
    std::io::stdout()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|e| format!("{STEP}: writing to standard output: {e}"))
}
