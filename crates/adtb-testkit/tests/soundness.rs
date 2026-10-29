//! Typing soundness: every value the reference returns for a well-typed term inhabits the type
//! the checker gave the term, and every row a statement inserts or deletes inhabits its row type.

use std::collections::BTreeMap;
use std::fs;
use std::ops::Range;
use std::path::Path;

use adtb_conformance as conformance;
use adtb_lang::ast::{AdhocItem, QueryDecl, TxnDecl};
use adtb_lang::check::{
    Bundles, Program, check_adhoc_recording, check_query, check_schema, check_txn,
};
use adtb_lang::kernel::{Terms, Tm, Var};
use adtb_lang::norm;
use adtb_lang::parse::parse_schema;
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::schema::{Schema, build};
use adtb_lang::span::FileId;
use adtb_lang::textual::Form;
use adtb_lang::types::{ModuleId, Ty};
use adtb_reference::{Abort, Database, Disk, Files, InMemory, Interpreter, Outcome, Supply, Value};
use adtb_testkit::cases::case;
use adtb_testkit::draw::draw;
use adtb_testkit::fixture::Fixture;
use adtb_testkit::fuzz::{self, Kind};
use adtb_testkit::inhabit::Judge;
use adtb_testkit::spec::{
    FRAGMENT, blocks, bundles, given_types, prelude_entries, prepare, registered_queries,
};
use adtb_testkit::{Rng, profile, running};

/// The rounds a `fix` may take before the statement is rejected.
const FIX_ROUNDS: usize = 200;

/// Returns the seeds of one run of `cases` cases as [`profile::deep`] sizes it, from
/// `ADTB_SOUND_SEED` and `ADTB_SOUND_CASES`.
fn seeds(cases: u64) -> Range<u64> {
    let var = |name: &str, default: u64| {
        std::env::var(name).map_or(default, |v| {
            v.parse()
                .unwrap_or_else(|_| panic!("{name} is not a number: {v}"))
        })
    };
    let first = var("ADTB_SOUND_SEED", 0);
    first..first + var("ADTB_SOUND_CASES", profile::deep(cases))
}

/// What a run judged and every judgement that failed.
#[derive(Default)]
struct Report {
    judged: usize,
    failures: Vec<String>,
}

impl Report {
    fn assert(&self, what: &str) {
        assert!(self.judged > 0, "no {what} was judged");
        assert!(
            self.failures.is_empty(),
            "{} of {} {what} do not inhabit their types:\n{}",
            self.failures.len(),
            self.judged,
            self.failures.join("\n")
        );
    }
}

/// The schema, files and state statements run against.
struct Run<'a> {
    schema: &'a Schema,
    program: &'a Program,
    files: &'a dyn Files,
    db: &'a Database,
}

impl Run<'_> {
    /// Judges outcome `o` of term `name`, of type `ty`, run against `prior`.
    fn judge(&self, name: &str, ty: &Ty, prior: &Database, o: &Outcome, out: &mut Report) {
        out.judged += 1;
        let after = Judge::over(self.schema, self.program, &o.db);
        let before = Judge::over(self.schema, self.program, prior);
        if let Err(m) = after.result(&o.result, ty)
            && before.result(&o.result, ty).is_err()
        {
            out.failures.push(format!("{name}: its result: {m}"));
        }
        let rows = [
            ("inserts", &o.change.inserted, after),
            ("deletes", &o.change.deleted, before),
        ];
        for (verb, change, judge) in rows {
            for (&(rel, ctor), rows) in change {
                for row in rows {
                    if let Err(m) = judge.row(rel, ctor, row) {
                        let r = &self.schema.relation(rel).name;
                        out.failures
                            .push(format!("{name}: it {verb} into `{r}`: {m}"));
                    }
                }
            }
        }
    }

    /// Runs `items` of `terms` in order, each named by `names`, and judges each that succeeds.
    /// Returns the state after the last when every item succeeds.
    fn items(
        &self,
        terms: &Terms,
        items: &[Tm],
        names: &[String],
        bound: &[(Var, Value)],
        supply: &mut Supply,
        out: &mut Report,
    ) -> Option<Database> {
        let mut db = self.db.clone();
        for (&item, name) in items.iter().zip(names) {
            let ty = match terms.ty(item) {
                Ty::Change => Ty::Unit,
                t => t.clone(),
            };
            let got = Interpreter::new(self.schema, self.program, &db)
                .with_fix_limit(FIX_ROUNDS)
                .with_files(self.files)
                .run(terms, item, bound, supply);
            match got {
                Ok(o) => {
                    self.judge(name, &ty, &db, &o, out);
                    db = o.db;
                }
                Err(a) => {
                    internal(name, &a, out);
                    return None;
                }
            }
        }
        Some(db)
    }
}

/// Records an abort that a well-typed term never reaches.
fn internal(name: &str, a: &Abort, out: &mut Report) {
    if let Abort::Internal(what) = a {
        out.judged += 1;
        out.failures.push(format!(
            "{name}: the reference aborts with `Internal`: {what}"
        ));
    }
}

/// The parameters of ad hoc text, each as name, type and value, and the bundles its `explain`
/// items name.
#[derive(Default)]
struct Given {
    params: Vec<(String, Ty, Value)>,
    bundles: Bundles,
}

/// Runs the ad hoc `items` of `source` in `fx`, in order and in their normal forms, over `given`,
/// a file scan reading `files`, and judges every outcome. When `keep`, the state after the last
/// item becomes the fixture's database. Returns whether the items check.
fn statements(
    fx: &mut Fixture,
    source: &str,
    items: &[AdhocItem],
    given: &Given,
    files: &dyn Files,
    keep: bool,
    out: &mut Report,
) -> bool {
    let params = &given.params;
    let tys: BTreeMap<String, Ty> = params
        .iter()
        .map(|(n, t, _)| (n.clone(), t.clone()))
        .collect();
    let Ok(adhoc) =
        check_adhoc_recording(&fx.schema, &fx.program, items, &tys, &given.bundles, None)
    else {
        return false;
    };
    let names: Vec<String> = items
        .iter()
        .filter_map(|i| match i {
            AdhocItem::Expr(e) => Some(e.span),
            AdhocItem::Write(w) => Some(w.span),
            AdhocItem::Transact(t) => Some(t.span),
            _ => None,
        })
        .map(|s| format!("`{}`", source[s.range()].trim()))
        .collect();
    let schema = adhoc.schema(&fx.schema);
    let run = Run {
        schema,
        program: &fx.program,
        files,
        db: &fx.db,
    };
    let bind = |vars: &BTreeMap<String, Var>| -> Vec<(Var, Value)> {
        params
            .iter()
            .map(|(n, _, v)| (vars[n], v.clone()))
            .collect()
    };
    if let Ok(normal) = norm::adhoc(schema, &fx.program, &adhoc) {
        let names: Vec<String> = names
            .iter()
            .map(|n| format!("the normal form of {n}"))
            .collect();
        let mut supply = fx.supply.clone();
        run.items(
            &normal.terms,
            &normal.items,
            &names,
            &bind(&normal.params),
            &mut supply,
            out,
        );
    }
    let mut supply = std::mem::take(&mut fx.supply);
    let after = run.items(
        &adhoc.terms,
        &adhoc.items,
        &names,
        &bind(&adhoc.params),
        &mut supply,
        out,
    );
    fx.supply = supply;
    if keep && let Some(mut db) = after {
        let declared = fx.schema.relations.len();
        db.retain(|(r, _), _| (r.0 as usize) < declared);
        fx.db = db;
    }
    true
}

/// Runs ad hoc text `text` in `fx` as [`statements`] does, without parameters. Returns whether
/// the text parses and checks.
fn text(fx: &mut Fixture, text: &str, keep: bool, out: &mut Report) -> bool {
    match parse_adhoc(text, Options::default()).ok() {
        Ok(items) => statements(
            fx,
            text,
            &items,
            &Given::default(),
            &InMemory::default(),
            keep,
            out,
        ),
        Err(_) => false,
    }
}

/// Returns a value drawn for each parameter type of `ty`, a `Lam` chain over `params`
/// parameters, and the type of its body; `None` when a parameter type has no textual form.
fn draw_args(schema: &Schema, ty: &Ty, params: usize, rng: &mut Rng) -> Option<(Vec<Value>, Ty)> {
    let mut ty = ty.clone();
    let mut args = Vec::new();
    for i in 0..params.max(1) {
        let Ty::Fn(a, body) = ty else {
            assert_eq!(i, 0, "a registered body is a `Lam` per parameter");
            break;
        };
        if params > 0 {
            let form = Form::of(schema, &a);
            if !form.has_syntax() {
                return None;
            }
            args.push(draw(rng, &form));
        }
        ty = *body;
    }
    Some((args, ty))
}

fn fixture_of(schema: Schema, program: Program) -> Fixture {
    Fixture {
        schema,
        program,
        db: Database::new(),
        supply: Supply::default(),
        closed: BTreeMap::new(),
        files: InMemory::default(),
    }
}

/// Calls registered query `q` of the bundle that reads `module` on arguments drawn from `rng`
/// and judges its result.
fn call_query(
    fx: &Fixture,
    module: ModuleId,
    q: &QueryDecl,
    at: &str,
    rng: &mut Rng,
    out: &mut Report,
) {
    let Ok(checked) = check_query(&fx.schema, &fx.program, module, q) else {
        return;
    };
    let ty = checked.terms.ty(checked.term);
    let Some((args, result)) = draw_args(&fx.schema, ty, q.params.len(), rng) else {
        return;
    };
    let name = format!("{at}: query {}", q.name.text);
    let got = Interpreter::new(&fx.schema, &fx.program, &fx.db)
        .with_fix_limit(FIX_ROUNDS)
        .apply(&checked.terms, checked.term, args);
    match got {
        Ok(v) => {
            out.judged += 1;
            let judge = Judge::over(&fx.schema, &fx.program, &fx.db);
            if let Err(m) = judge.result(&v, &result) {
                out.failures.push(format!("{name}: its result: {m}"));
            }
        }
        Err(a) => internal(&name, &a, out),
    }
}

/// Calls registered transaction `t` of the bundle that reads `module` on arguments drawn from
/// `rng` and judges its outcome.
fn call_txn(
    fx: &Fixture,
    module: ModuleId,
    t: &TxnDecl,
    at: &str,
    rng: &mut Rng,
    out: &mut Report,
) {
    let Ok(checked) = check_txn(&fx.schema, &fx.program, module, t) else {
        return;
    };
    let ty = checked.terms.ty(checked.term);
    let Some((args, result)) = draw_args(&fx.schema, ty, t.params.len(), rng) else {
        return;
    };
    let name = format!("{at}: transaction {}", t.name.text);
    let run = Run {
        schema: &fx.schema,
        program: &fx.program,
        files: &fx.files,
        db: &fx.db,
    };
    let got = Interpreter::new(&fx.schema, &fx.program, &fx.db)
        .with_fix_limit(FIX_ROUNDS)
        .run_call(&checked.terms, checked.term, args, &mut fx.supply.clone());
    match got {
        Ok(o) => run.judge(&name, &result, &fx.db, &o, out),
        Err(a) => internal(&name, &a, out),
    }
}

/// Runs every ad hoc item, registered query and registered transaction of the spec's examples
/// on a database that starts empty, the items in order, each parameter drawn from `rng`.
#[test]
fn spec_corpus_results_inhabit_their_types() {
    let blocks = blocks();
    let base = running::version_12(&blocks);
    let parts = running::parts(&blocks, &base);
    let prelude = prelude_entries();
    let registered = registered_queries(&blocks);
    let mut out = Report::default();
    let mut rng = Rng::new(0);
    for b in blocks.iter().filter(|b| b.is_adt() && b.info != FRAGMENT) {
        let part = parts.get(b.file.as_str()).unwrap_or(&base);
        let Ok(p) = prepare(b, part, &prelude) else {
            continue;
        };
        let at = format!("{}:{}", b.file, b.line);
        let (tys, unresolved) = given_types(b, &p.schema);
        let mut params = Vec::new();
        for (name, ty) in tys {
            let form = Form::of(&p.schema, &ty);
            if form.has_syntax() {
                params.push((name, ty, draw(&mut rng, &form)));
            }
        }
        let (queries, txns, module, adhoc) = (p.queries, p.txns, p.module, p.adhoc);
        let mut fx = fixture_of(p.schema, p.program);
        for q in &queries {
            call_query(&fx, module, q, &at, &mut rng, &mut out);
        }
        for t in &txns {
            call_txn(&fx, module, t, &at, &mut rng, &mut out);
        }
        if unresolved.is_empty() && !adhoc.is_empty() {
            let given = Given {
                params,
                bundles: bundles(&fx.schema, &registered).0,
            };
            statements(
                &mut fx,
                &b.text,
                &adhoc,
                &given,
                &InMemory::default(),
                true,
                &mut out,
            );
        }
    }
    out.assert("spec examples");
}

/// Cases per run of the generated cases.
const CASES: u64 = 24;

/// The queries of each generated case.
const QUERIES: usize = 12;

/// Runs each generated case's load and every query it draws, with its partitions by constructor
/// and by predicate and its counts.
#[test]
fn generated_case_results_inhabit_their_types() {
    let mut out = Report::default();
    for seed in seeds(CASES) {
        let c = case(seed, QUERIES);
        let src = c
            .schema
            .strip_prefix("module app\n")
            .expect("a case's schema is module `app`");
        let mut fx = Fixture::new(src);
        let before = out.failures.len();
        for w in &c.load {
            assert!(
                text(&mut fx, w, true, &mut out),
                "seed {seed}: `{w}` checks"
            );
        }
        for q in &c.queries {
            let mut texts = vec![q.text()];
            texts.extend(q.by_ctor().into_iter().flatten());
            if let Some(split) = q.by_predicate() {
                texts.extend(split.parts);
            }
            if let Some(counts) = q.counts() {
                texts.push(counts.whole);
                texts.extend(counts.parts);
            }
            for t in &texts {
                assert!(
                    text(&mut fx, t, false, &mut out),
                    "seed {seed}: `{t}` checks"
                );
            }
        }
        for f in &mut out.failures[before..] {
            *f = format!("seed {seed}: {f}");
        }
    }
    out.assert("generated statements");
}

/// The rows the fuzzed statements range over, one write a line.
const FUZZ_LOAD: &str = r#"insert items { name: "a", n: 1, status: Draft }
insert items { name: "b", n: 2, status: Sent { at: 5 } }
insert items { name: "c", n: -3, status: Paid { at: 7, by: "x" } }
insert tags.Small { item: items.key(1), size: 3 }
insert tags.Large { item: items.key(2), label: "l" }"#;

/// Seeds per run of the fuzzed statements; about one in four draws unmutated ad hoc text.
const FUZZ_SEEDS: u64 = 2000;

/// Runs every unmutated ad hoc statement the front-end fuzzer draws, over a database of a few rows.
#[test]
fn fuzzed_statement_results_inhabit_their_types() {
    let src = fuzz::SCHEMA
        .strip_prefix("module app\n")
        .expect("the fuzz schema is module `app`");
    let mut fx = Fixture::new(src);
    let mut out = Report::default();
    for w in FUZZ_LOAD.lines() {
        assert!(text(&mut fx, w, true, &mut out), "`{w}` checks");
    }
    let (db, supply) = (fx.db.clone(), fx.supply.clone());
    let mut checked = 0;
    for seed in seeds(FUZZ_SEEDS) {
        let c = fuzz::case(seed, &[]);
        if c.kind != Kind::Adhoc || c.mutated {
            continue;
        }
        let before = out.failures.len();
        checked += usize::from(text(&mut fx, &c.text, true, &mut out));
        for f in &mut out.failures[before..] {
            *f = format!("seed {seed}: {f}");
        }
        fx.db.clone_from(&db);
        fx.supply.clone_from(&supply);
    }
    assert!(checked > 0, "no fuzzed statement checks");
    out.assert("fuzzed statements");
}

/// Returns the schema whose module files `body` holds, each starting with its `module` line.
fn conformance_schema(body: &str) -> Option<Fixture> {
    let mut texts: Vec<String> = Vec::new();
    for line in body.lines() {
        if line.starts_with("module ") || texts.is_empty() {
            texts.push(String::new());
        }
        let cur = texts.last_mut().expect("a module");
        cur.push_str(line);
        cur.push('\n');
    }
    texts.retain(|m| !m.trim().is_empty());
    let mut files = Vec::new();
    for (i, text) in texts.iter().enumerate() {
        files.push((
            FileId(i as u32),
            parse_schema(text, Options::default()).ok().ok()?,
        ));
    }
    let schema = build(&files).ok()?;
    let program = check_schema(&schema).ok()?;
    Some(fixture_of(schema, program))
}

/// Runs the statements of every script of the conformance suite in order, each script on a
/// database that starts empty, keeping the state of each `run`.
#[test]
fn conformance_results_inhabit_their_types() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
    let mut out = Report::default();
    for path in conformance::scripts(&dir) {
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let Ok(script) = conformance::parse(&text) else {
            continue;
        };
        let files = Disk::new(path.parent().expect("a script is in a directory"));
        let mut fx = None;
        for d in &script.directives {
            let before = out.failures.len();
            if d.kind == conformance::Kind::Schema {
                fx = conformance_schema(&d.body);
                if fx.is_none() {
                    break;
                }
                continue;
            }
            let fx = match &mut fx {
                Some(f) => f,
                None => fx.insert(conformance_schema("").expect("the prelude checks")),
            };
            let keep = d.kind == conformance::Kind::Run;
            if let Ok(items) = parse_adhoc(&d.body, Options::default()).ok() {
                statements(
                    fx,
                    &d.body,
                    &items,
                    &Given::default(),
                    &files,
                    keep,
                    &mut out,
                );
            }
            for f in &mut out.failures[before..] {
                *f = format!("{}:{}: {f}", path.display(), d.line);
            }
        }
    }
    out.assert("conformance statements");
}
