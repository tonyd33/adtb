//! The conformance suite on the engine, over the simulator.
//!
//! Each file runs in a process of the `adtb-conformance` binary, so a file that overflows the
//! stack, aborts or does not finish fails alone.

use std::path::{Path, PathBuf};
use std::thread;

use adtb_conformance::runner::{self, Corpus, Format, Options, Sides};
use adtb_lang::canon::canonical_term;
use adtb_lang::evolve::Source;
use adtb_lang::parse::{parse_migration, parse_schema};
use adtb_lang::span::{FileId, Span};
use adtb_testkit::Rng;
use adtb_testkit::fuzz::{Kind, case, spec_corpus};
use adtb_testkit::profile::{self, assert_reached};
use adtb_testkit::running;
use adtb_testkit::spec::blocks;

/// How many files run at once.
const MOST_WORKERS: usize = 8;

/// The first fuzz seed and the number of fuzz cases drawn, a quarter of them schemas.
const FUZZ_SEED: u64 = 0x5eed_0000;
const FUZZ_CASES: u64 = 400;

fn suite() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn conforms(part: &str) {
    let corpus = Corpus::new(Format::Conformance, suite());
    let dir = suite().join(part);
    let jobs = thread::available_parallelism().map_or(4, |n| n.get().min(MOST_WORKERS));
    let program = PathBuf::from(env!("CARGO_BIN_EXE_adtb-conformance"));
    let options = Options::new(Sides::Engine, jobs).isolated(program);
    let files = runner::select(&corpus, &dir).cases().len();
    let passed = runner::passes(&corpus, &dir, &options).unwrap_or_else(|out| panic!("{out}"));
    assert_eq!(files, passed, "{part}");
}

#[test]
fn model() {
    conforms("model");
}

#[test]
fn queries() {
    conforms("queries");
}

#[test]
fn writes() {
    conforms("writes");
}

#[test]
fn constraints() {
    conforms("constraints");
}

#[test]
fn conformance() {
    conforms("conformance");
}

#[test]
fn interop() {
    conforms("interop");
}

/// Returns whether each migration of schema `body` is accepted, recording it as the engine does.
///
/// Fail when an accepted operator does not resolve, or a declaration of the source the
/// migrations leave writes a name the source does not resolve.
fn spelled(body: &str) -> Result<bool, String> {
    let mut source = Source::default();
    let Ok(texts) = adtb_conformance::migrations(body) else {
        return Ok(false);
    };
    for text in texts {
        let Ok(m) = parse_migration(&text, adtb_lang::parse::Options::default()).ok() else {
            return Ok(false);
        };
        let Ok((next, _, recorded)) = source.record(&m, &text, true) else {
            return Ok(false);
        };
        if let Err(ds) = recorded.resolved() {
            return Err(format!("the operators do not resolve: {ds:?}"));
        }
        source = next;
    }
    let names = source.names().map_err(|ds| format!("{ds:?}"))?;
    for (i, names) in names.iter().enumerate() {
        let text = source.file_text(FileId(i as u32));
        canonical_term(&text, Span::new(0, text.len()), names).map_err(|u| u.to_string())?;
    }
    Ok(true)
}

/// Returns the spec's running schema at version 12 and each part's extension of it, as schema
/// bodies.
fn spec_bodies() -> Vec<(String, String)> {
    let mut bodies: Vec<(String, String)> = Vec::new();
    let blocks = blocks();
    let base = running::version_12(&blocks);
    let mut schemas = vec![("version 12".to_string(), base.clone())];
    schemas.extend(
        running::parts(&blocks, &base)
            .into_iter()
            .map(|(part, entries)| (part.to_string(), entries)),
    );
    for (name, entries) in schemas {
        let mut modules: Vec<&str> = Vec::new();
        for e in &entries {
            if !modules.contains(&e.module.as_str()) {
                modules.push(&e.module);
            }
        }
        let mut body = String::new();
        for m in modules {
            body.push_str(&format!("module {m}\n"));
            for e in entries.iter().filter(|e| e.module == m) {
                body.push_str(&e.text);
                body.push('\n');
            }
        }
        bodies.push((format!("spec {name}"), body));
    }
    bodies
}

#[test]
fn every_name_an_accepted_schema_writes_resolves() {
    let mut bodies = spec_bodies();
    let corpus = spec_corpus(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec"));
    for seed in FUZZ_SEED..FUZZ_SEED + profile::deep(FUZZ_CASES) {
        let c = case(seed, &corpus);
        if c.kind == Kind::Schema {
            bodies.push((format!("fuzz seed {seed}"), c.text));
        }
    }
    let jobs = thread::available_parallelism().map_or(4, |n| n.get().min(MOST_WORKERS));
    let results: Vec<Result<bool, String>> = thread::scope(|s| {
        let chunks: Vec<_> = bodies
            .chunks(bodies.len().div_ceil(jobs))
            .map(|chunk| s.spawn(|| chunk.iter().map(|(_, b)| spelled(b)).collect::<Vec<_>>()))
            .collect();
        chunks
            .into_iter()
            .flat_map(|h| h.join().expect("a worker finishes"))
            .collect()
    });
    let (mut spec, mut fuzz) = (0, 0);
    let mut failures = Vec::new();
    for ((name, _), result) in bodies.iter().zip(&results) {
        match result {
            Ok(true) if name.starts_with("spec ") => spec += 1,
            Ok(true) => fuzz += 1,
            Ok(false) => {}
            Err(why) => failures.push(format!("{name}: {why}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} schemas fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_reached("accepted spec schemas", spec, 8);
    assert_reached("accepted fuzz schemas", fuzz, 30);
}

/// Returns each module file of `body`.
fn module_files(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in body.lines() {
        if line.starts_with("module ") || out.is_empty() {
            out.push(String::new());
        }
        let cur = out.last_mut().expect("a module");
        cur.push_str(line);
        cur.push('\n');
    }
    out
}

/// Returns the schema bodies of every conformance file under `dir`.
fn conformance_bodies(dir: &Path, out: &mut Vec<(String, String)>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("the suite is readable")
        .map(|e| e.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            conformance_bodies(&path, out);
        } else if path.extension().is_some_and(|e| e == "test") {
            let text = std::fs::read_to_string(&path).expect("a readable conformance file");
            let Ok(script) = adtb_conformance::parse(&text) else {
                continue;
            };
            for d in script.directives {
                if d.kind == adtb_conformance::Kind::Schema {
                    out.push((format!("{}:{}", path.display(), d.line), d.body));
                }
            }
        }
    }
}

/// How replaying a schema body ends: the sorted declarations of the source it leaves, or the
/// reason it is refused.
#[derive(Debug, PartialEq, Eq)]
enum Replayed {
    Accepted(Vec<String>),
    Refused(String),
}

/// Returns how the migrations `adtb_conformance::migrations` gives for `body` end, applied in
/// turn, or why a migration it gives is refused.
fn replayed(body: &str) -> Result<Replayed, String> {
    let texts = match adtb_conformance::migrations(body) {
        Ok(texts) => texts,
        Err(why) => return Ok(Replayed::Refused(why)),
    };
    let mut source = Source::default();
    for text in texts {
        let m = parse_migration(&text, adtb_lang::parse::Options::default())
            .ok()
            .map_err(|ds| format!("{text}\ndoes not parse: {ds:?}"))?;
        source = source
            .accept(&m, &text)
            .map_err(|r| format!("{text}\nrefused: {:?} {:?}", r.operators, r.recheck))?;
    }
    let mut decls: Vec<String> = source.decls.into_iter().map(|d| d.text).collect();
    decls.sort();
    Ok(Replayed::Accepted(decls))
}

/// Returns how replaying `body` ends when it is accepted: its declarations as written, sorted;
/// else the reason `adtb_conformance::migrations` refuses it.
fn written(body: &str) -> Replayed {
    if let Err(why) = adtb_conformance::migrations(body) {
        return Replayed::Refused(why);
    }
    let mut decls = Vec::new();
    for text in module_files(body) {
        let file = parse_schema(&text, adtb_lang::parse::Options::default())
            .ok()
            .expect("a replayed module file parses");
        decls.extend(file.decls.iter().map(|d| text[d.span().range()].to_owned()));
    }
    decls.sort();
    Replayed::Accepted(decls)
}

/// Returns `body` with the declarations of each module file in the order `order` gives for
/// the number of them.
fn permuted(body: &str, order: &mut dyn FnMut(usize) -> Vec<usize>) -> String {
    let mut out = String::new();
    for text in module_files(body) {
        let file = parse_schema(&text, adtb_lang::parse::Options::default())
            .ok()
            .expect("a replayed module file parses");
        out.push_str(&format!("module {}\n", file.module.text));
        for i in order(file.decls.len()) {
            out.push_str(&text[file.decls[i].span().range()]);
            out.push('\n');
        }
    }
    out
}

/// Returns every order of `n` items.
fn every_order(n: usize) -> Vec<Vec<usize>> {
    fn all(prefix: &mut Vec<usize>, n: usize, out: &mut Vec<Vec<usize>>) {
        if prefix.len() == n {
            out.push(prefix.clone());
            return;
        }
        for i in 0..n {
            if !prefix.contains(&i) {
                prefix.push(i);
                all(prefix, n, out);
                prefix.pop();
            }
        }
    }
    let mut out = Vec::new();
    all(&mut Vec::new(), n, &mut out);
    out
}

/// Returns the orders of the declarations of `body` tried: each module file reversed, and under
/// the deep profile every order of a single module file of at most `PERMUTED_ALL` declarations,
/// else `PERMUTED_DRAWS` shuffles seeded by `seed`.
fn orders(body: &str, seed: u64) -> Vec<String> {
    let mut out = vec![permuted(body, &mut |n| (0..n).rev().collect())];
    if !profile::is("deep") {
        return out;
    }
    let sizes: Vec<usize> = module_files(body)
        .iter()
        .map(|t| {
            parse_schema(t, adtb_lang::parse::Options::default())
                .ok()
                .map_or(0, |f| f.decls.len())
        })
        .collect();
    if let [n] = sizes[..]
        && n <= PERMUTED_ALL
    {
        for o in every_order(n) {
            out.push(permuted(body, &mut |_| o.clone()));
        }
        return out;
    }
    let mut rng = Rng::new(seed);
    for _ in 0..PERMUTED_DRAWS {
        out.push(permuted(body, &mut |n| {
            let mut p: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                p.swap(i, rng.below(i as u64 + 1) as usize);
            }
            p
        }));
    }
    out
}

/// The most declarations a module file has for every order of them to be tried.
const PERMUTED_ALL: usize = 4;

/// The seeded orders tried of a body with more declarations.
const PERMUTED_DRAWS: u64 = 24;

/// The seed of the shuffles of the first body.
const PERMUTED_SEED: u64 = 0x0de5_0000;

/// A type whose domain names a constructor of a type that holds it.
const CYCLIC: &str = "module demo
type B = X { p: Pos } | Y
type Pos = new I64 where self > 0 and Y == Y
type R = { id: I64, b: B }
relation bs : R key id
";

/// A fn named like a field, before the relation it reads and that relation's type.
const NAMED_LIKE_A_FIELD: &str = "module demo
relation pets : Pet key name
type Pet = { name: Text }
fn name () -> I64 = count(1 for p in pets)
";

#[test]
fn a_schema_body_replays_alike_in_every_order_of_its_declarations() {
    assert_eq!(
        Ok(Replayed::Refused(
            "module demo: the declaration needs itself through the declarations it names".into()
        )),
        replayed(CYCLIC)
    );
    assert_eq!(
        Ok(Replayed::Accepted(vec![
            "fn name () -> I64 = count(1 for p in pets)".into(),
            "relation pets : Pet key name".into(),
            "type Pet = { name: Text }".into(),
        ])),
        replayed(NAMED_LIKE_A_FIELD)
    );
    for (body, n) in [(CYCLIC, 4), (NAMED_LIKE_A_FIELD, 3)] {
        for o in every_order(n) {
            let p = permuted(body, &mut |_| o.clone());
            assert_eq!(replayed(body), replayed(&p), "{p}");
        }
    }
    let mut bodies = spec_bodies();
    conformance_bodies(&suite(), &mut bodies);
    let jobs = thread::available_parallelism().map_or(4, |n| n.get().min(MOST_WORKERS));
    let results: Vec<(usize, usize, Vec<String>)> = thread::scope(|s| {
        let workers: Vec<_> = (0..jobs)
            .map(|k| {
                let bodies = &bodies;
                s.spawn(move || {
                    let (mut accepted, mut tried) = (0, 0);
                    let mut failures = Vec::new();
                    for (i, (name, body)) in bodies.iter().enumerate().skip(k).step_by(jobs) {
                        let expected = written(body);
                        if let Replayed::Accepted(_) = expected {
                            accepted += 1;
                        }
                        for p in orders(body, PERMUTED_SEED + i as u64) {
                            tried += 1;
                            match replayed(&p) {
                                Ok(got) if got == expected => {}
                                Ok(got) => failures.push(format!(
                                    "{name}: in the order\n{p}ends as {got:?}, not {expected:?}"
                                )),
                                Err(why) => failures.push(format!("{name}: {why}")),
                            }
                        }
                    }
                    (accepted, tried, failures)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|h| h.join().expect("a worker finishes"))
            .collect()
    });
    let failures: Vec<&str> = results
        .iter()
        .flat_map(|r| r.2.iter().map(String::as_str))
        .collect();
    assert!(
        failures.is_empty(),
        "{} bodies are refused or differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
    let accepted: usize = results.iter().map(|r| r.0).sum();
    let tried: usize = results.iter().map(|r| r.1).sum();
    assert_reached("schema bodies accepted", accepted, 280);
    assert_reached("orders replayed", tried, 280);
}
