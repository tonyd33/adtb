//! Cases for differential testing mutated from the conformance suite: the schema and writes of a
//! script, and its queries with mutants of them that still type-check.

use std::collections::BTreeSet;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

use adtb_lang::ast::AdhocItem;
use adtb_lang::parse::{Options, parse_adhoc};

use crate::Rng;
use crate::cases::Case;
use crate::fixture::Fixture;

/// A conformance script of one module whose writes the reference accepts.
#[derive(Clone, Debug)]
pub struct Script {
    pub name: String,
    /// The declarations of the module, without its `module` line.
    decls: String,
    load: Vec<String>,
    queries: Vec<String>,
}

/// The words a query of a case holds none of: writes, ordering, and file scans, whose files a
/// case does not hold.
const EXCLUDED: [&str; 11] = [
    "insert",
    "delete",
    "update",
    "order",
    "take",
    "top",
    "transact",
    "read_csv",
    "read_ndjson",
    "read_parquet",
    "read_arrow",
];

/// The generated values a script's results may hold, which the engine and the reference agree
/// on only up to a bijection.
const UNCOMPARED: [&str; 2] = ["generate clock", "generate uuid"];

/// The attempts at a mutant a case may make per query it asks for.
const ATTEMPTS: usize = 12;

/// Returns the scripts under `dir` and its subdirectories that a case can be made of, by name.
pub fn scripts(dir: &Path) -> Vec<Script> {
    let mut paths = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let entries = std::fs::read_dir(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display()));
        for entry in entries {
            let path = entry
                .unwrap_or_else(|e| panic!("{}: {e}", d.display()))
                .path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|x| x == "test") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
        .iter()
        .filter_map(|p| {
            let text =
                std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            let name = p.strip_prefix(dir).unwrap_or(p).display().to_string();
            Script::read(name, &text)
        })
        .collect()
}

impl Script {
    /// Reads a script, or returns `None` when its schema is not one module or generates values
    /// other than sequences, it holds no query a case can ask, or the reference refuses its
    /// schema or a write.
    fn read(name: String, text: &str) -> Option<Script> {
        let mut schema = None;
        let mut load = Vec::new();
        let mut queries = Vec::new();
        for (head, body) in directives(text) {
            match head.split_whitespace().next() {
                Some("schema") => schema = Some(body),
                Some("run") => load.extend(statements(&body)?),
                Some("query") if allowed(&body) => queries.push(body.trim().to_owned()),
                _ => {}
            }
        }
        let schema = schema?;
        let mut modules = schema.lines().filter(|l| l.starts_with("module "));
        let (Some(first), None) = (modules.next(), modules.next()) else {
            return None;
        };
        let decls = schema.replacen(&format!("{first}\n"), "", 1);
        if UNCOMPARED.iter().any(|g| decls.contains(g)) {
            return None;
        }
        let script = Script {
            name,
            decls,
            load,
            queries,
        };
        let fx = script.reference()?;
        let checked: Vec<String> = script
            .queries
            .iter()
            .filter(|q| fx.checks(q))
            .cloned()
            .collect();
        (!checked.is_empty()).then_some(Script {
            queries: checked,
            ..script
        })
    }

    /// Returns the reference holding the script's database, or `None` when it refuses the
    /// schema or a write.
    fn reference(&self) -> Option<Fixture> {
        panic::catch_unwind(AssertUnwindSafe(|| {
            let mut fx = Fixture::new(&self.decls);
            self.load.iter().all(|w| fx.run(w).is_ok()).then_some(fx)
        }))
        .ok()
        .flatten()
    }

    /// Returns the case of this script and `seed`: its schema as module `app`, its writes, and
    /// `n` queries, its own first, then mutants of them that type-check.
    pub fn case(&self, seed: u64, n: usize) -> Case {
        let fx = self
            .reference()
            .expect("a script's reference accepts its schema and writes");
        let mut rng = Rng::new(seed);
        let pool: Vec<Vec<Token>> = self.queries.iter().map(|q| tokens(q)).collect();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut out: Vec<String> = Vec::new();
        for q in self.queries.iter().take(n / 2) {
            if seen.insert(q.clone()) {
                out.push(q.clone());
            }
        }
        for _ in 0..n * ATTEMPTS {
            if out.len() >= n {
                break;
            }
            let base = rng.pick(&pool).clone();
            let rounds = 1 + rng.below(2);
            let mutant = (0..rounds).fold(base, |t, _| mutate(&mut rng, t, &pool));
            let text = render(&mutant);
            if !seen.contains(&text) && fx.checks(&text) {
                seen.insert(text.clone());
                out.push(text);
            }
        }
        let schema = format!("module app\n{}", self.decls);
        Case::of_text(schema, migration(&self.decls), self.load.clone(), out)
    }
}

/// Returns whether a query body holds no write, ordering, file scan or parameter.
fn allowed(body: &str) -> bool {
    let mut words = body.split(|c: char| !c.is_alphanumeric() && c != '_');
    !body.contains('$') && !words.any(|w| EXCLUDED.contains(&w))
}

/// Returns each directive of a script, its head line and its body with the indentation of its
/// first line removed.
fn directives(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        let word = line.split_whitespace().next().unwrap_or_default();
        if !line.starts_with(' ') && ["schema", "run", "query", "error"].contains(&word) {
            out.push((line.to_owned(), String::new()));
            open = true;
        } else if line == "----" || (!line.starts_with(' ') && !line.is_empty()) {
            open = false;
        } else if open && let Some((_, body)) = out.last_mut() {
            body.push_str(line.strip_prefix("  ").unwrap_or(line));
            body.push('\n');
        }
    }
    out
}

/// Returns the statements of a body, or `None` when it does not parse as writes, expressions
/// and transaction blocks.
fn statements(body: &str) -> Option<Vec<String>> {
    let items = parse_adhoc(body, Options::default()).ok().ok()?;
    let mut starts = Vec::new();
    for item in &items {
        let span = match item {
            AdhocItem::Expr(e) => e.span,
            AdhocItem::Write(w) => w.span,
            AdhocItem::Transact(t) => t.span,
            _ => return None,
        };
        starts.push(span.start as usize);
    }
    starts.push(body.len());
    Some(
        starts
            .windows(2)
            .map(|w| body[w[0]..w[1]].trim().to_owned())
            .collect(),
    )
}

/// Returns the migration adding module `app` with `decls`, each declaration in order, each
/// relation of a type the module declares moved up to follow its type.
fn migration(decls: &str) -> String {
    let mut items: Vec<String> = Vec::new();
    for line in decls.lines() {
        let indented = line.starts_with(' ') || line.starts_with("--") || line.trim().is_empty();
        match items.last_mut() {
            Some(d) if indented => {
                d.push_str(line);
                d.push('\n');
            }
            _ if indented => {}
            _ => items.push(format!("{line}\n")),
        }
    }
    let word = |d: &str, i: usize| d.split_whitespace().nth(i).unwrap_or_default().to_owned();
    let declared = |d: &str| {
        (word(d, 0) == "type").then(|| {
            word(d, 1)
                .split(['[', '=', '{'])
                .next()
                .unwrap_or_default()
                .to_owned()
        })
    };
    let types: Vec<String> = items.iter().filter_map(|d| declared(d)).collect();
    let moved = |d: &str| word(d, 0) == "relation" && types.contains(&word(d, 3));
    let mut out = String::from("module app\nadd module app\n");
    for d in items.iter().filter(|d| !moved(d)) {
        out.push_str(match word(d, 0).as_str() {
            "fn" | "aggregate" => "define ",
            _ => "add ",
        });
        out.push_str(d);
        if let Some(ty) = declared(d) {
            for r in items.iter().filter(|r| moved(r) && word(r, 3) == ty) {
                out.push_str("add ");
                out.push_str(r);
            }
        }
    }
    out
}

/// A token of a query and the whitespace before it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    space: String,
    text: String,
}

fn tokens(text: &str) -> Vec<Token> {
    let cs: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let start = i;
        while i < cs.len() && cs[i].is_whitespace() {
            i += 1;
        }
        let space: String = cs[start..i].iter().collect();
        if i == cs.len() {
            break;
        }
        let begin = i;
        let c = cs[i];
        if c == '"' {
            i += 1;
            while i < cs.len() && cs[i] != '"' {
                i += if cs[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(cs.len());
        } else if c.is_alphanumeric() || c == '_' {
            while i < cs.len()
                && (cs[i].is_alphanumeric()
                    || cs[i] == '_'
                    || (cs[i] == '.'
                        && c.is_ascii_digit()
                        && cs.get(i + 1).is_some_and(char::is_ascii_digit)))
            {
                i += 1;
            }
        } else {
            let two: String = cs[i..(i + 2).min(cs.len())].iter().collect();
            i += if ["==", "!=", "<=", ">=", "->", "++"].contains(&two.as_str()) {
                2
            } else {
                1
            };
        }
        out.push(Token {
            space,
            text: cs[begin..i].iter().collect(),
        });
    }
    out
}

fn render(ts: &[Token]) -> String {
    let mut out = String::new();
    for t in ts {
        out.push_str(&t.space);
        out.push_str(&t.text);
    }
    out.trim().to_owned()
}

/// The pairs of operators a mutation swaps.
const SWAPS: [(&str, &str); 5] = [
    ("==", "!="),
    ("<", ">="),
    ("<=", ">"),
    ("and", "or"),
    ("+", "-"),
];

/// Returns `ts` with one mutation: an integer moved, an operator swapped, a name or a text
/// literal replaced by another the queries hold, or a parenthesized subterm replaced by one of
/// another query.
fn mutate(rng: &mut Rng, mut ts: Vec<Token>, pool: &[Vec<Token>]) -> Vec<Token> {
    let at = |ts: &[Token], f: &dyn Fn(&str) -> bool| -> Vec<usize> {
        (0..ts.len()).filter(|&i| f(&ts[i].text)).collect()
    };
    let swap = |t: &str| SWAPS.iter().find(|(a, b)| *a == t || *b == t);
    match rng.below(5) {
        0 => {
            if let Some(&k) = rng.pick_some(&at(&ts, &|t| t.chars().all(|c| c.is_ascii_digit()))) {
                let n: i64 = ts[k].text.parse().unwrap_or(0);
                if let Some(m) = n.checked_add(*rng.pick(&[-1, 1, 2])) {
                    ts[k].text = m.max(0).to_string();
                }
            }
        }
        1 => {
            if let Some(&k) = rng.pick_some(&at(&ts, &|t| swap(t).is_some())) {
                let (a, b) = swap(&ts[k].text).expect("a swappable operator");
                ts[k].text = if *a == ts[k].text {
                    (*b).into()
                } else {
                    (*a).into()
                };
            }
        }
        2 | 3 => {
            let class = if rng.below(2) == 0 { "name" } else { "text" };
            let others: Vec<&Token> = pool
                .iter()
                .flatten()
                .filter(|t| kind(&t.text) == class)
                .collect();
            if let (Some(&k), Some(t)) = (
                rng.pick_some(&at(&ts, &|t| kind(t) == class)),
                rng.pick_some(&others),
            ) {
                ts[k].text = t.text.clone();
            }
        }
        _ => {
            let mine = groups(&ts);
            let donor = rng.pick(pool);
            let theirs = groups(donor);
            if let (Some(&(a, b)), Some(&(c, d))) = (rng.pick_some(&mine), rng.pick_some(&theirs)) {
                let mut graft = donor[c..d].to_vec();
                graft[0].space = ts[a].space.clone();
                ts.splice(a..b, graft);
            }
        }
    }
    ts
}

/// Returns the token ranges of each parenthesized subterm of `ts`.
fn groups(ts: &[Token]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut open = Vec::new();
    for (i, t) in ts.iter().enumerate() {
        match t.text.as_str() {
            "(" => open.push(i),
            ")" => out.extend(open.pop().map(|s| (s, i + 1))),
            _ => {}
        }
    }
    out
}

/// The words a name replacement leaves alone.
const KEYWORDS: [&str; 22] = [
    "from", "in", "where", "select", "match", "key", "fix", "union", "group", "by", "into", "if",
    "then", "else", "not", "and", "or", "try", "let", "for", "exists", "with",
];

/// Returns the class of a token a replacement keeps: a text literal, a number, a name, or the
/// token itself.
fn kind(t: &str) -> String {
    match t.chars().next() {
        Some('"') => "text".into(),
        Some(c) if c.is_ascii_digit() => "number".into(),
        Some(c) if (c.is_alphabetic() || c == '_') && !KEYWORDS.contains(&t) => "name".into(),
        _ => t.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_the_largest_integer_keeps_it_an_integer() {
        let ts = tokens("9223372036854775807");
        let pool = vec![ts.clone()];
        for seed in 0..64 {
            let mut rng = Rng::new(seed);
            let moved = panic::catch_unwind(AssertUnwindSafe(|| {
                render(&mutate(&mut rng, ts.clone(), &pool))
            }));
            assert!(
                moved.as_ref().is_ok_and(|t| t.parse::<i64>().is_ok()),
                "seed {seed}: {moved:?}"
            );
        }
    }
}
