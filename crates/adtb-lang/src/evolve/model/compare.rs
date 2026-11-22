use std::cell::RefCell;
use std::io::Write as _;
use std::sync::{Mutex, OnceLock, PoisonError};

use super::apply::Migrated;
use super::op::{Fail, Refused};
use super::*;
use crate::ast::{self, MigrationFile, OperatorKind};
use crate::evolve::{Recorded, Refusal, ResolvedOp, RowMap, SourceDecl, meaning};
use crate::ids::{KeyKind, ObjectKey, Owner, is_named};
use crate::lex::{Tok, lex};
use crate::span::Span;

/// Returns the tokens of `text`, comments and the end left out.
fn tokens(text: &str) -> Vec<String> {
    lex(text)
        .0
        .into_iter()
        .filter(|t| t.tok != Tok::Eof)
        .map(|t| text[t.span.range()].to_string())
        .collect()
}

thread_local! {
    /// The outcomes of the comparisons [`forced`] runs on this thread, by their first word.
    static FORCED: RefCell<Option<BTreeMap<String, usize>>> = const { RefCell::new(None) };
}

/// Returns whether migrations are compared: under the nextest profile `deep`, read once, or
/// inside [`forced`].
pub(in crate::evolve) fn enabled() -> bool {
    static DEEP: OnceLock<bool> = OnceLock::new();
    FORCED.with(|f| f.borrow().is_some())
        || *DEEP.get_or_init(|| std::env::var("NEXTEST_PROFILE").is_ok_and(|p| p == "deep"))
}

/// Run `f` comparing every migration it applies on this thread. Returns its result and the
/// number of comparisons of each hook and outcome, such as `Migrating agreed` or
/// `Recorded skipped`.
#[cfg(test)]
pub(in crate::evolve) fn forced<T>(f: impl FnOnce() -> T) -> (T, BTreeMap<String, usize>) {
    FORCED.with(|x| *x.borrow_mut() = Some(BTreeMap::new()));
    let out = f();
    let tally = FORCED.with(|x| x.borrow_mut().take()).expect("the tally");
    (out, tally)
}

/// A place that compares a migration the operator layer applies.
#[derive(Clone, Copy, Debug)]
enum Hook {
    /// [`compare`], from `Source::migrating`.
    Migrating,
    /// [`compare_recorded`], from `Source::record`.
    Recorded,
}

/// The comparisons a hook made in this process: how many, and how many it skipped.
#[derive(Clone, Copy, Debug, Default)]
struct Tally {
    total: usize,
    skipped: usize,
}

/// The comparisons [`judge`] counted of each hook in this process, by [`Hook`].
static TALLIES: Mutex<[Tally; 2]> = Mutex::new(
    [Tally {
        total: 0,
        skipped: 0,
    }; 2],
);

/// The share of a hook's comparisons in one process that may be skipped, as a fraction.
const SKIPPED_AT_MOST: (usize, usize) = (1, 10);

/// The fewest comparisons of a hook in one process [`SKIPPED_AT_MOST`] judges. Below it the
/// process must have compared a migration before it skips one.
const JUDGED_FROM: usize = SKIPPED_AT_MOST.1;

/// The tests whose skips [`SKIPPED_AT_MOST`] does not judge, by the name of the thread that
/// runs them. Each applies few migrations, among them one the model does not compare.
const UNJUDGED: &[&str] = &[];

/// Count `line`'s outcome for `hook` and for [`forced`], and append it to the file
/// `ADTB_MIGRATION_MODEL_LOG` names, when it names one. A `partial` of [`Hook::Migrating`]
/// compares nothing and counts as skipped. Judge it unless its thread is one of [`UNJUDGED`].
fn log(hook: Hook, line: &str) {
    let outcome = line.split(['\t', ' ']).next().unwrap_or(line);
    let skipped = outcome == "skipped" || (outcome == "partial" && matches!(hook, Hook::Migrating));
    FORCED.with(|x| {
        if let Some(tally) = x.borrow_mut().as_mut() {
            let key = if skipped {
                format!("{hook:?} skipped")
            } else {
                format!("{hook:?} {outcome}")
            };
            *tally.entry(key).or_default() += 1;
        }
    });
    let thread = std::thread::current();
    let name = thread.name().unwrap_or_default();
    if !UNJUDGED.contains(&name) {
        judge(hook, skipped, name, line);
    }
    let Some(path) = std::env::var_os("ADTB_MIGRATION_MODEL_LOG") else {
        return;
    };
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap_or_else(|e| panic!("the migration model log {path:?} opens: {e}"));
    f.write_all(format!("{line}\n").as_bytes())
        .unwrap_or_else(|e| panic!("the migration model log {path:?} takes a line: {e}"));
}

/// Count a comparison of `hook` on thread `name`, `skipped` or not, whose log line is `line`.
///
/// Panics:
/// - `hook` has made [`JUDGED_FROM`] comparisons in this process and skipped more than
///   [`SKIPPED_AT_MOST`] of them.
/// - `hook` has made fewer and skipped one before it compared one.
fn judge(hook: Hook, skipped: bool, name: &str, line: &str) {
    let tally = {
        let mut all = TALLIES.lock().unwrap_or_else(PoisonError::into_inner);
        let t = &mut all[hook as usize];
        t.total += 1;
        t.skipped += usize::from(skipped);
        *t
    };
    let (num, den) = SKIPPED_AT_MOST;
    if tally.total >= JUDGED_FROM {
        assert!(
            tally.skipped * den <= tally.total * num,
            "the {hook:?} hook skipped {} of its {} comparisons in this process, more than {num}/{den}, on thread {name:?}; the last: {line}",
            tally.skipped,
            tally.total,
        );
    } else {
        assert!(
            tally.skipped < tally.total,
            "the {hook:?} hook skipped a migration in this process before it compared one, on thread {name:?}: {line}"
        );
    }
}

/// Apply migration `m` to the model of `before`, keeping the constraints and checks of `kept`,
/// and panic where the model fails or the result differs from `after`, the operator layer's
/// result, by a difference no known defect of that layer explains.
pub(in crate::evolve) fn compare(
    before: &Source,
    m: &MigrationFile,
    text: &str,
    kept: &[crate::evolve::Kept],
    accepting: bool,
    after: Result<&Source, &Refusal>,
) {
    let model = lifted(before);
    let migrated = model
        .migrate_keeping(m, text, kept, accepting, false)
        .unwrap_or_else(|why| panic!("the migration model fails on migration\n{text}\n{why}"));
    if let Some(kind) = migrated.stopped() {
        let outcome = if migrated.ops().len() == m.operators.len() {
            "partial"
        } else {
            "skipped"
        };
        log(Hook::Migrating, &format!("{outcome}\t{kind}"));
        return;
    }
    let diffs = differences(m, &migrated, after);
    report(Hook::Migrating, m, text, &migrated, diffs, after, "agreed");
}

/// Log a migration the model and the layer agree on as `agreed`, and one whose `diffs` a
/// known defect explains as that defect; panic on any other.
fn report(
    hook: Hook,
    m: &MigrationFile,
    text: &str,
    got: &Migrated,
    diffs: Vec<Diff>,
    after: Result<&Source, &Refusal>,
    agreed: &str,
) {
    if diffs.is_empty() {
        log(hook, agreed);
        return;
    }
    match known(m, got, &diffs, after) {
        Some(finding) => log(hook, &format!("known\t{finding}")),
        None => panic!(
            "the migration model and the operator layer differ on migration\n{text}\n{}",
            diffs
                .iter()
                .map(|d| d.shown.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

/// Returns the model of `before`, panicking unless it prints and places every object as
/// `before` holds it.
fn lifted(before: &Source) -> Model {
    let model =
        of(before).unwrap_or_else(|e| panic!("the model does not lift a source that checks: {e}"));
    lifted_as_held(&model, before);
    model
}

/// Panic unless `model`, lifted from `before`, prints its declarations and places its objects
/// as `before` holds them.
fn lifted_as_held(model: &Model, before: &Source) {
    let printed: Vec<String> = model
        .source_decls()
        .expect("a lifted model names only held objects")
        .iter()
        .flat_map(|d| tokens(&d.text))
        .collect();
    let held: Vec<String> = before.decls.iter().flat_map(|d| tokens(&d.text)).collect();
    assert_eq!(
        printed.join(" "),
        held.join(" "),
        "the model prints the declarations as the source holds them"
    );
    let built = model
        .schema(&[])
        .unwrap_or_else(|e| panic!("the model's declarations build: {e:?}"));
    let held = before.schema().expect("a source being migrated builds");
    assert_eq!(
        built.ids(),
        held.ids(),
        "the model places every object of its declarations at the id the source gives it"
    );
}

/// Returns, for each operator of `m`, the reasons a refusal gives for it, each with its span.
fn refused_ops(m: &MigrationFile, r: &Refusal) -> Vec<Vec<(String, Span)>> {
    m.operators
        .iter()
        .map(|op| {
            r.operators
                .iter()
                .filter(|d| op.span.start <= d.span.start && d.span.end <= op.span.end)
                .map(|d| (d.message.clone(), d.span))
                .collect()
        })
        .collect()
}

/// Returns whether the layer's reasons `layer` hold the model's refusal `mine`: its reason at its
/// span.
fn holds_refusal(layer: &[(String, Span)], mine: &Refused) -> bool {
    layer
        .iter()
        .any(|(why, at)| why == mine.why() && mine.span() == Some(*at))
}

/// What a difference is in.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Aspect {
    /// An operator both refuse, for reasons that differ.
    Reason,
    /// Which operators are refused; `recheck_only` when the layer refuses at its recheck alone
    /// and the model applies every operator to declarations that check.
    Refusals {
        recheck_only: bool,
    },
    /// The declarations, of these modules.
    Declarations(BTreeSet<String>),
    /// The version and the log.
    Log,
    Modules,
    States,
    ValueStates,
    Grants,
    NextIds,
    NamedIds,
    ClauseIds,
    /// The id of each object of the source the model gives, by its key.
    SourceIds,
    /// The functions and aggregates of the result.
    Functions,
    /// An operator's steps or row maps.
    Steps,
    /// The names of the migration's text.
    Names,
    Resolved,
}

/// A difference between the model's result and the operator layer's, and how it shows.
struct Diff {
    aspect: Aspect,
    shown: String,
}

/// Returns each way the model's result differs from the operator layer's.
fn differences(m: &MigrationFile, got: &Migrated, after: Result<&Source, &Refusal>) -> Vec<Diff> {
    let model_refused: Vec<bool> = got
        .ops()
        .iter()
        .map(|o| o.as_ref().is_err_and(Refused::reported))
        .collect();
    let mut out = Vec::new();
    match after {
        Err(r) => {
            let reasons = refused_ops(m, r);
            let held: Vec<bool> = reasons.iter().map(|x| !x.is_empty()).collect();
            if held != model_refused || (!held.contains(&true) && got.checks()) {
                let recheck_only =
                    !held.contains(&true) && !model_refused.contains(&true) && got.checks();
                out.push(Diff {
                    aspect: Aspect::Refusals { recheck_only },
                    shown: format!(
                        "refused operators: layer {held:?} (recheck {}), model {model_refused:?} (checks {})",
                        r.recheck.len(),
                        got.checks()
                    ),
                });
            }
            for (k, (layer, mine)) in reasons.iter().zip(got.ops()).enumerate() {
                if let Err(why) = mine
                    && why.reported()
                    && !layer.is_empty()
                    && !holds_refusal(layer, why)
                {
                    out.push(Diff {
                        aspect: Aspect::Reason,
                        shown: format!("operator {k} reason:\n  layer {layer:?}\n  model {why:?}"),
                    });
                }
            }
        }
        Ok(src) => {
            if model_refused.contains(&true) || !got.checks() {
                out.push(Diff {
                    aspect: Aspect::Refusals {
                        recheck_only: false,
                    },
                    shown: format!(
                        "the layer applies the migration; the model refuses {:?}, checks {}: {}",
                        got.ops()
                            .iter()
                            .filter_map(|o| o.as_ref().err())
                            .collect::<Vec<_>>(),
                        got.checks(),
                        got.model().unchecked()
                    ),
                });
                return out;
            }
            out.extend(same(got.model(), src, reverts(m)));
            out.extend(functions_differ(got.model(), src));
        }
    }
    out
}

/// Returns how the functions and aggregates of the source `model` gives differ from those of
/// `src`, each with its signature and canonical code.
fn functions_differ(model: &Model, src: &Source) -> Option<Diff> {
    let shown = |s: &Source| match s.functions() {
        Ok(fs) => format!("{fs:?}"),
        Err(ds) => format!("{:?}", ds.first().map(|d| &d.diag.message)),
    };
    let (layer, mine) = (shown(src), shown(&model.source()));
    (layer != mine).then(|| Diff {
        aspect: Aspect::Functions,
        shown: format!("Functions:\n  layer {layer}\n  model {mine}"),
    })
}

/// Returns how declarations `mine` differ from `layer`'s in meaning, each compared by its
/// tokens with [`meaning::decls`].
fn meaning_differs(layer: &[SourceDecl], mine: &[SourceDecl]) -> Option<Diff> {
    let (layer, mine) = (meaning::decls(layer), meaning::decls(mine));
    if layer == mine {
        return None;
    }
    let modules: BTreeSet<String> = layer
        .iter()
        .chain(&mine)
        .filter(|d| !layer.contains(d) || !mine.contains(d))
        .map(|(m, _)| m.clone())
        .collect();
    let aspect = Aspect::Declarations(modules);
    let shown = format!("{aspect:?}:\n  layer {layer:?}\n  model {mine:?}");
    Some(Diff { aspect, shown })
}

/// Returns how the ids of the source `model` gives differ from those `src` holds, by key.
fn source_ids_differ(model: &Model, src: &Source) -> Option<Diff> {
    let given = model.source().ids;
    let layer: Vec<_> = src
        .ids
        .iter()
        .filter(|(k, v)| given.get(k) != Some(v))
        .collect();
    let mine: Vec<_> = given
        .iter()
        .filter(|(k, v)| src.ids.get(k) != Some(v))
        .collect();
    if layer.is_empty() && mine.is_empty() {
        return None;
    }
    let shown = format!("SourceIds:\n  layer {layer:?}\n  model {mine:?}");
    Some(Diff {
        aspect: Aspect::SourceIds,
        shown,
    })
}

/// Returns whether `m` holds a `revert`.
fn reverts(m: &MigrationFile) -> bool {
    m.operators
        .iter()
        .any(|op| matches!(op.kind, OperatorKind::Revert { .. }))
}

/// Returns each way `model` differs from `src`, the declarations compared by their meaning when
/// `by_meaning`. While `src` does not build, its clause ids may still hold those of clauses that
/// are gone, so the model's need only be among them.
fn same(model: &Model, src: &Source, by_meaning: bool) -> Vec<Diff> {
    let printed_decls = match model.printed() {
        Ok(ds) => ds,
        Err(why) => {
            return vec![Diff {
                aspect: Aspect::Declarations(BTreeSet::new()),
                shown: why,
            }];
        }
    };
    let mut out: Vec<Diff> = if by_meaning {
        meaning_differs(&src.decls, &printed_decls)
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    let mut check = |aspect: Aspect, a: String, b: String| {
        if a != b {
            let shown = format!("{aspect:?}:\n  layer {a}\n  model {b}");
            out.push(Diff { aspect, shown });
        }
    };
    let module_tokens = |ds: &[SourceDecl]| -> Vec<String> {
        ds.iter()
            .flat_map(|d| {
                tokens(&d.text)
                    .into_iter()
                    .map(|t| format!("{}: {t}", d.module))
            })
            .collect()
    };
    let printed = module_tokens(&printed_decls);
    let held = module_tokens(&src.decls);
    if !by_meaning && held != printed {
        let of = |ds: &[SourceDecl], m: &str| -> Vec<String> {
            ds.iter()
                .filter(|d| d.module == m)
                .flat_map(|d| tokens(&d.text))
                .collect()
        };
        let mine = &printed_decls;
        let modules: BTreeSet<String> = src
            .decls
            .iter()
            .chain(mine)
            .map(|d| d.module.clone())
            .filter(|m| of(&src.decls, m) != of(mine, m))
            .collect();
        let at = held
            .iter()
            .zip(&printed)
            .take_while(|(a, b)| a == b)
            .count();
        let from = at.saturating_sub(8);
        check(
            Aspect::Declarations(modules),
            held[from..]
                .iter()
                .take(40)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
            printed[from..]
                .iter()
                .take(40)
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    check(
        Aspect::Log,
        format!(
            "{} {:?}",
            src.version,
            src.log.iter().map(LogM::of).collect::<Vec<_>>()
        ),
        format!("{} {:?}", model.version, model.log),
    );
    check(
        Aspect::Modules,
        format!("{:?} {:?}", src.modules, src.installed),
        format!("{:?} {:?}", model.modules, model.installed),
    );
    let states: BTreeMap<(u64, u64), State> = src
        .states
        .iter()
        .map(|(&(r, c), &s)| ((r.raw(), c.raw()), s))
        .collect();
    check(
        Aspect::States,
        format!("{states:?}"),
        format!("{:?}", model.states),
    );
    let values: BTreeMap<u64, State> = src
        .value_states
        .iter()
        .map(|(&c, &s)| (c.raw(), s))
        .collect();
    check(
        Aspect::ValueStates,
        format!("{values:?}"),
        format!("{:?}", model.value_states),
    );
    check(
        Aspect::Grants,
        format!("{:?}", src.grants),
        format!("{:?}", model.grants),
    );
    check(
        Aspect::NextIds,
        format!("{:?}", src.next),
        format!("{:?}", model.next),
    );
    let named: BTreeMap<&ObjectKey, u64> = src
        .ids
        .iter()
        .filter(|(k, _)| is_named(k.kind()))
        .map(|(k, &v)| (k, v))
        .collect();
    let mine = model.named_keys();
    check(
        Aspect::NamedIds,
        format!("{named:?}"),
        format!("{:?}", mine.iter().collect::<BTreeMap<_, _>>()),
    );
    let clauses: BTreeSet<(Space, Owner, u64)> = src
        .ids
        .iter()
        .filter(|(k, _)| !is_named(k.kind()))
        .map(|(k, &v)| (k.kind().space(), k.owner().clone(), v))
        .collect();
    let mine = model.clause_ids();
    let builds = src.schema().is_ok();
    if builds || !mine.is_subset(&clauses) {
        check(
            Aspect::ClauseIds,
            format!("{clauses:?}"),
            format!("{mine:?}"),
        );
    }
    if builds {
        out.extend(source_ids_differ(model, src));
    }
    out
}

impl Model {
    /// Returns the key and id of each named object, as a source holds them.
    pub(super) fn named_keys(&self) -> BTreeMap<ObjectKey, u64> {
        let mut out = self.held_ids.clone();
        let shared = |id: u64| Owner::Object(Space::Shared, id);
        for d in &self.decls {
            let module = || Owner::Module(d.module.clone());
            match d.item {
                Item::Type(t) => {
                    out.insert(
                        ObjectKey::named(KeyKind::Type, module(), &self.types[&t].name),
                        t,
                    );
                    for c in self.ctors_of(t) {
                        out.insert(
                            ObjectKey::named(KeyKind::Ctor, shared(t), &self.ctors[&c].name),
                            c,
                        );
                        for f in self.ctors[&c].fields.iter().flatten() {
                            out.insert(
                                ObjectKey::named(KeyKind::Field, shared(c), &self.fields[f].name),
                                *f,
                            );
                        }
                    }
                    for f in self.types[&t].header.iter().flatten() {
                        out.insert(
                            ObjectKey::named(KeyKind::Field, shared(t), &self.fields[f].name),
                            *f,
                        );
                    }
                }
                Item::Relation(r) => {
                    let rel = &self.relations[&r];
                    out.insert(ObjectKey::named(KeyKind::Relation, module(), &rel.name), r);
                    for c in &rel.clauses {
                        if let ClauseM::Computed {
                            id: Some(id), name, ..
                        } = c
                        {
                            out.insert(ObjectKey::named(KeyKind::Computed, shared(r), name), *id);
                        }
                    }
                }
                Item::Set(s) => {
                    let set = &self.sets[&s];
                    out.insert(
                        ObjectKey::named(KeyKind::Set, shared(set.relation), &set.name),
                        s,
                    );
                }
                Item::Fn(f) | Item::Aggregate(f) => {
                    out.insert(
                        ObjectKey::named(KeyKind::Function, module(), &self.functions[&f].name),
                        f,
                    );
                }
                Item::Constraint(c) | Item::Check(c) => {
                    out.insert(
                        ObjectKey::named(KeyKind::Constraint, module(), &self.denials[&c].name),
                        c,
                    );
                }
                Item::Store(v) => {
                    let st = &self.stores[&v];
                    let name = self.store_name(v);
                    out.insert(ObjectKey::named(KeyKind::Relation, module(), name), v);
                    if let Some(&t) = st.ids.get(&ViewSlot::Type) {
                        out.insert(ObjectKey::named(KeyKind::Type, shared(v), name), t);
                        if let Some(&c) = st.ids.get(&ViewSlot::Ctor) {
                            out.insert(ObjectKey::named(KeyKind::Ctor, shared(t), name), c);
                            for (slot, &f) in &st.ids {
                                if let ViewSlot::Field(n) = slot {
                                    out.insert(ObjectKey::named(KeyKind::Field, shared(c), n), f);
                                }
                            }
                        }
                    }
                }
                Item::Held(..) => {}
            }
        }
        out
    }

    /// Returns the space, owner and id of each key, unique, index, implied index and default.
    fn clause_ids(&self) -> BTreeSet<(Space, Owner, u64)> {
        let mut out = BTreeSet::new();
        for (&r, rel) in &self.relations {
            let owner = Owner::Object(Space::Shared, r);
            let mut put = |space, id: Option<u64>| {
                if let Some(id) = id {
                    out.insert((space, owner.clone(), id));
                }
            };
            for c in &rel.clauses {
                match c {
                    ClauseM::Key { id, generate, .. } => {
                        put(Space::Index, *id);
                        put(Space::Default, generate.as_ref().and_then(|g| g.1));
                    }
                    ClauseM::Index { id, .. } => put(Space::Index, *id),
                    ClauseM::Default { ids, .. } => {
                        for &(_, d) in ids {
                            put(Space::Default, Some(d));
                        }
                    }
                    ClauseM::Computed { .. } | ClauseM::Volatile => {}
                }
            }
            for &i in rel.implied.values() {
                put(Space::Index, Some(i));
            }
        }
        for (&v, st) in &self.stores {
            for (slot, &id) in &st.ids {
                if matches!(slot, ViewSlot::Key | ViewSlot::Index(_)) {
                    out.insert((Space::Index, Owner::Object(Space::Shared, v), id));
                }
            }
        }
        out
    }
}

/// Returns a map as the comparison reads it: its kind, positions and fields, and the normal
/// form of its row function.
fn map_summary(m: &RowMap) -> String {
    match m {
        RowMap::Fill {
            def,
            ctors,
            field,
            fill,
        } => format!("fill {} {ctors:?} {field} {:?}", def.0, fill.query),
        RowMap::Copy { def, ctors, field } => format!("copy {} {ctors:?} {field}", def.0),
        RowMap::Route { def, from, routing } => {
            format!("route {} {from:?} {:?}", def.0, routing.query)
        }
        RowMap::Merge {
            def,
            from,
            into,
            discriminator,
        } => format!("merge {} {from:?} {into} {discriminator}", def.0),
        RowMap::Nest { def, from, into } => format!("nest {} {from:?} {into}", def.0),
        RowMap::Unnest { def, from } => format!("unnest {} {from}", def.0),
        RowMap::Rekey { relation } => format!("rekey {}", relation.0),
    }
}

/// Apply migration `m` to the model of `before`, and panic where the model fails, or where an
/// operator's step count, row maps or source after it, the resolved operators, or the result,
/// differ from `recorded` and `after`, the operator layer's, by a difference no known defect of
/// that layer explains. Operators are compared up to the first one the model does not apply.
/// Returns the model's resolved operators when it applies every operator and agrees.
pub(in crate::evolve) fn compare_recorded(
    before: &Source,
    m: &MigrationFile,
    text: &str,
    accepting: bool,
    after: &Source,
    recorded: &Recorded,
) -> Option<Vec<ResolvedOp>> {
    let steps = recorded.steps();
    let model = lifted(before);
    let got = model
        .migrate_keeping(m, text, &[], accepting, true)
        .unwrap_or_else(|why| panic!("the migration model fails on migration\n{text}\n{why}"));
    let mut diffs = Vec::new();
    let applied = got.ops().len();
    let end = got
        .maps()
        .iter()
        .position(Result::is_err)
        .map_or(applied, |k| k + 1);
    let whole = got.stopped().is_none() || end < applied;
    if steps.len() < end || (whole && steps.len() != end) {
        diffs.push(Diff {
            aspect: Aspect::Steps,
            shown: format!(
                "the layer has steps for {} operators, the model for {end}",
                steps.len()
            ),
        });
    }
    for (k, (held, mine)) in steps.iter().zip(got.maps()).enumerate() {
        let mut step = |shown: String| {
            diffs.push(Diff {
                aspect: Aspect::Steps,
                shown: format!("operator {k}: {shown}"),
            });
        };
        if matches!(m.operators[k].kind, OperatorKind::Revert { .. }) {
            match (held, mine, got.steps()[k].as_ref()) {
                (Ok(held), Ok(_), Some(Ok(mine))) => {
                    diffs.extend(revert_steps_differ(k, held, mine, &got.afters()[k]));
                }
                (Err(ds), Err(why), _) if ds.iter().any(|d| d.message == *why) => {}
                (held, mine, _) => {
                    let mine = mine.as_ref().map(Vec::len);
                    step(format!("layer steps {held:?}, model maps {mine:?}"));
                }
            }
            continue;
        }
        let held = match (held, mine) {
            (Ok(held), Ok(mine)) if held.len() == 1 => {
                let layer: Vec<String> = held[0].maps.iter().map(map_summary).collect();
                let mine: Vec<String> = mine.iter().map(map_summary).collect();
                if layer != mine {
                    step(format!("maps:\n  layer {layer:#?}\n  model {mine:#?}"));
                }
                &held[0]
            }
            (Err(ds), Err(why)) => {
                if !ds.iter().any(|d| d.message == *why) {
                    step(format!("steps fail:\n  layer {ds:?}\n  model {why:?}"));
                }
                continue;
            }
            (held, mine) => {
                let mine = mine.as_ref().map(Vec::len);
                step(format!("layer steps {held:?}, model maps {mine:?}"));
                continue;
            }
        };
        for d in same(&got.afters()[k], &held.after, false) {
            diffs.push(Diff {
                shown: format!("after operator {k}, {}", d.shown),
                ..d
            });
        }
    }
    let unavailable = model.unavailable(m);
    if got.stopped().is_none() {
        diffs.extend(differences(m, &got, Ok(after)));
        diffs.extend(names_differ(before, m, text, &got));
        let mine: Vec<ResolvedOp> = got.ops().iter().flatten().cloned().collect();
        let layer = recorded.resolved().map_err(|ds| {
            ds.iter()
                .map(|d| (d.span, d.message.clone()))
                .collect::<Vec<_>>()
        });
        let agrees = match &layer {
            Ok(held) => unavailable.is_empty() && *held == mine.as_slice(),
            Err(ds) => *ds == unavailable,
        };
        if !agrees {
            diffs.push(Diff {
                aspect: Aspect::Resolved,
                shown: format!(
                    "resolved:\n  layer {layer:?}\n  model {mine:?}, unavailable {unavailable:?}"
                ),
            });
        }
    }
    let agreed = match got.stopped() {
        Some(kind) if applied == 0 => format!("skipped\t{kind}"),
        Some(kind) => format!("partial\t{kind}"),
        None => "agreed steps".to_string(),
    };
    let whole = got.stopped().is_none() && diffs.is_empty() && unavailable.is_empty();
    report(Hook::Recorded, m, text, &got, diffs, Ok(after), &agreed);
    whole.then(|| got.ops().iter().flatten().cloned().collect())
}

/// Returns how the steps `mine` of `revert` operator `k`, which leaves `after`, differ from the
/// layer's steps `held`: in their row maps and operators, or in the afters a step's maps are
/// stated against or a mapped step leaves.
fn revert_steps_differ(
    k: usize,
    held: &[crate::evolve::Step],
    mine: &[revert::Stepped],
    after: &Model,
) -> Vec<Diff> {
    let summary = |maps: &[RowMap], applied: Option<(&str, &str)>| {
        let maps: Vec<String> = maps.iter().map(map_summary).collect();
        format!("{maps:?} {applied:?}")
    };
    let layer: Vec<String> = held
        .iter()
        .map(|s| {
            let applied = s
                .applied
                .as_ref()
                .map(|a| (a.module.as_str(), &a.text[a.op.span.range()]));
            summary(&s.maps, applied)
        })
        .collect();
    let mut model: Vec<String> = mine
        .iter()
        .map(|s| summary(s.maps(), s.applied()))
        .collect();
    if model.is_empty() {
        model.push(summary(&[], None));
    }
    if layer != model {
        return vec![Diff {
            aspect: Aspect::Steps,
            shown: format!("operator {k}: steps:\n  layer {layer:#?}\n  model {model:#?}"),
        }];
    }
    let afters = mine
        .iter()
        .map(revert::Stepped::after)
        .take(held.len().saturating_sub(1))
        .chain(std::iter::once(after));
    let last = held.len() - 1;
    // An after is read only by the maps of the step after it, or as a mapped step's or the
    // operator's result.
    let read = |j: usize| j == last || !held[j].maps.is_empty() || !held[j + 1].maps.is_empty();
    let mut out = Vec::new();
    for (j, (layer, mine)) in held.iter().zip(afters).enumerate() {
        if !read(j) {
            continue;
        }
        let within = |a: &Aspect| {
            j == last
                || !matches!(
                    a,
                    Aspect::States | Aspect::ValueStates | Aspect::Grants | Aspect::Log
                )
        };
        for d in same(mine, &layer.after, true)
            .into_iter()
            .filter(|d| within(&d.aspect))
        {
            out.push(Diff {
                shown: format!("after step {j} of operator {k}, {}", d.shown),
                ..d
            });
        }
    }
    out
}

/// Returns how the names the model gives the text of `m`, applied to `before`, differ from those
/// [`Logged::names`] gives it.
fn names_differ(before: &Source, m: &MigrationFile, text: &str, got: &Migrated) -> Option<Diff> {
    let mine = got.names()?;
    let logged = crate::evolve::Logged {
        number: before.version + 1,
        module: m.module.text.clone(),
        text: text.to_string(),
        operators: m.operators.clone(),
        before: std::sync::Arc::new(before.clone()),
    };
    let layer = logged.names();
    let shown = |r: &Result<crate::canon::Names, Vec<crate::span::Diag>>| match r {
        Ok(n) => format!("{n:?}"),
        Err(ds) => format!(
            "{:?}",
            ds.iter().map(|d| (d.span, &d.message)).collect::<Vec<_>>()
        ),
    };
    let (layer, mine) = (shown(&layer), shown(mine));
    (layer != mine).then(|| Diff {
        aspect: Aspect::Names,
        shown: format!("Names:\n  layer {layer}\n  model {mine}"),
    })
}

/// Returns the finding of the operator layer that explains `diffs`, when one does; `layer` is
/// the source the layer leaves, when it leaves one.
fn known(
    m: &MigrationFile,
    got: &Migrated,
    diffs: &[Diff],
    after: Result<&Source, &Refusal>,
) -> Option<&'static str> {
    let layer = after.ok();
    if let Some(finding) = shared_field_refused(m, got, diffs, after.err()) {
        return Some(finding);
    }
    if let Some(finding) = respelled_constructor(m, got, diffs, layer) {
        return Some(finding);
    }
    if let Some(finding) = moved_without_action(m, got, diffs, layer) {
        return Some(finding);
    }
    let renamed: BTreeSet<&str> = m
        .operators
        .iter()
        .filter_map(|op| match &op.kind {
            OperatorKind::RenameConstructor(p, _) => Some(p.ty.name.text.as_str()),
            _ => None,
        })
        .collect();
    let twin = |other: &String| {
        *other != m.module.text
            && renamed
                .iter()
                .any(|t| got.model().type_named(other, t).is_some())
    };
    let twin_only = |modules: &BTreeSet<String>| modules.iter().all(twin);
    let twinned = got.model().modules.iter().any(twin);
    let e1 = !renamed.is_empty()
        && diffs.iter().all(|d| match &d.aspect {
            Aspect::Declarations(ms) => twin_only(ms),
            Aspect::Refusals { recheck_only } => *recheck_only && twinned,
            _ => false,
        });
    e1.then_some(
        "E-1: rename constructor rewrites a selection of a same-named type of another module",
    )
}

/// Returns the finding when `diffs` are only in declarations and ids, and renaming, after
/// each `rename type T to U` of `m`, a constructor `T` of a type that is not a record to `U`
/// gives the model's result as `layer`.
fn respelled_constructor(
    m: &MigrationFile,
    got: &Migrated,
    diffs: &[Diff],
    layer: Option<&Source>,
) -> Option<&'static str> {
    let layer = layer?;
    if !diffs.iter().all(|d| {
        matches!(
            d.aspect,
            Aspect::Declarations(_) | Aspect::NamedIds | Aspect::SourceIds
        )
    }) {
        return None;
    }
    let mut model = got.model().clone();
    let mut respelled = false;
    for op in &m.operators {
        let OperatorKind::RenameType(q, to) = &op.kind else {
            continue;
        };
        let module = q.module.as_ref().map_or(&m.module.text, |x| &x.text);
        let Some(t) = model.type_named(module, &to.text) else {
            continue;
        };
        if !matches!(model.types[&t].body, TypeBody::Ctors(_)) {
            continue;
        }
        if let Some(c) = model.ctor_named(t, &q.name.text) {
            model.ctors.get_mut(&c).expect("the constructor").name = to.text.clone();
            respelled = true;
        }
    }
    (respelled && same(&model, layer, false).is_empty())
        .then_some("E-2: rename type also renames a constructor of a sum spelled like the type")
}

/// Returns the finding when `diffs` are only in declarations, and dropping the delete action of
/// each field a `move field` of `m` moved gives the model's result as `layer`.
fn moved_without_action(
    m: &MigrationFile,
    got: &Migrated,
    diffs: &[Diff],
    layer: Option<&Source>,
) -> Option<&'static str> {
    let layer = layer?;
    if !diffs
        .iter()
        .all(|d| matches!(d.aspect, Aspect::Declarations(_)))
    {
        return None;
    }
    let mut model = got.model().clone();
    let mut dropped = false;
    for op in &m.operators {
        let OperatorKind::MoveField { path, sel, .. } = &op.kind else {
            continue;
        };
        let module = path.ty.module.as_ref().map_or(&m.module.text, |x| &x.text);
        let Some(t) = model.type_named(module, &path.ty.name.text) else {
            continue;
        };
        let parts: Vec<Option<u64>> = match sel {
            None => vec![None],
            Some(ast::CtorSel::One(c)) => vec![model.ctor_named(t, &c.text)],
            Some(ast::CtorSel::Many(cs)) => {
                cs.iter().map(|c| model.ctor_named(t, &c.text)).collect()
            }
        };
        for part in parts {
            if let Some(f) = model.field_of(t, part, &path.field.text)
                && let Some(held) = model.fields.get_mut(&f)
            {
                dropped |= held.action.take().is_some();
            }
        }
    }
    (dropped && same(&model, layer, false).is_empty())
        .then_some("F7: move field leaves the moved field `on delete restrict`")
}

/// Returns the finding when the layer refuses only at its recheck, each reason `no `f` in scope`
/// for a field `f` of an `add unique` or `add index` of `m` over a constructor set, and the model
/// either applies the migration, or refuses only `add unique` operators, each because a member of
/// its set lacks the field or declares it at another type.
fn shared_field_refused(
    m: &MigrationFile,
    got: &Migrated,
    diffs: &[Diff],
    refusal: Option<&Refusal>,
) -> Option<&'static str> {
    let refusal = refusal?;
    if !refusal.operators.is_empty()
        || refusal.recheck.is_empty()
        || !diffs
            .iter()
            .all(|d| matches!(d.aspect, Aspect::Refusals { .. }))
    {
        return None;
    }
    let mut scoped = BTreeSet::new();
    for (op, mine) in m.operators.iter().zip(got.ops()) {
        let (OperatorKind::AddUnique(fields, on) | OperatorKind::AddIndex { fields, on, .. }) =
            &op.kind
        else {
            if mine.is_err() {
                return None;
            }
            continue;
        };
        if on.sel.is_none() {
            continue;
        }
        if let Err(r) = mine {
            let member = r.why().starts_with("member `")
                && (r.why().contains("` has no field `") || r.why().contains("` declares `"));
            if !matches!(op.kind, OperatorKind::AddUnique(..)) || !member {
                return None;
            }
        }
        for f in fields {
            if let ast::ExprKind::Name(q) = &f.kind {
                scoped.insert(format!("no `{}` in scope", q.name.text));
            }
        }
    }
    (!scoped.is_empty()
        && refusal
            .recheck
            .iter()
            .all(|d| scoped.contains(&d.diag.message)))
    .then_some("F6: a unique or index over a field of a constructor set is refused only at the recheck, as no field in scope")
}

/// Panic when the operator layer gives the inverses `layer` of a logged migration that the
/// model's replay fails as `failed`, the revert being of migration text `text`.
pub(super) fn replay_failed(
    text: &str,
    layer: &Result<crate::evolve::revert::Inverted, Vec<crate::span::Diag>>,
    failed: &Fail,
) {
    if layer.is_ok() {
        panic!(
            "the model's replay of the reverted migration fails where the operator layer's applies, in revert\n{text}\n  model {failed:?}"
        );
    }
}

/// Panic unless the inverses `mine`, from the model's replay of a logged migration, are those
/// `layer`, from the operator layer's, gives, the revert being of migration text `text`.
pub(super) fn same_inverses(
    text: &str,
    layer: &Result<crate::evolve::revert::Inverted, Vec<crate::span::Diag>>,
    mine: &Result<crate::evolve::revert::Inverted, Vec<crate::span::Diag>>,
) {
    let refused = |ds: &Vec<crate::span::Diag>| {
        format!(
            "{:?}",
            ds.iter().map(|d| (d.span, &d.message)).collect::<Vec<_>>()
        )
    };
    let differ = match (layer, mine) {
        (Ok(a), Ok(b)) => a.differs_from(b),
        (Err(a), Err(b)) => {
            let (a, b) = (refused(a), refused(b));
            (a != b).then(|| ("refusal".into(), a, b))
        }
        (Ok(_), Err(b)) => Some(("refusal".into(), "none".into(), refused(b))),
        (Err(a), Ok(_)) => Some(("refusal".into(), refused(a), "none".into())),
    };
    if let Some((part, layer, mine)) = differ {
        panic!(
            "the model's inverses differ from the operator layer's in revert\n{text}\n{part}:\n  layer {layer}\n  model {mine}"
        );
    }
    FORCED.with(|x| {
        if let Some(tally) = x.borrow_mut().as_mut() {
            *tally.entry("Inverses agreed".into()).or_default() += 1;
        }
    });
}
