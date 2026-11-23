//! Every clause a migration rewrites covers the image of the rows it covered, the image taken by
//! the generator's model of each operator's effect on rows, never by the operator's own output.

use std::sync::Mutex;

use adtb_lang::canon::Names;

use super::*;

/// The kinds whose operators drop, re-key or re-act on the objects the judge compares; a
/// migration holding one is not judged.
const UNJUDGED: [Kind; 12] = [
    Kind::DropRelation,
    Kind::DropSet,
    Kind::DropField,
    Kind::DropUnique,
    Kind::DropIndex,
    Kind::Rekey,
    Kind::DropDefault,
    Kind::AlterField,
    Kind::DropCheck,
    Kind::DropComputed,
    Kind::DropConstraint,
    Kind::Script,
];

/// An object of a relation that covers rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Obj {
    Key,
    Unique,
    Index,
    Set,
    Check,
    Default,
    Computed,
    /// A reference field's delete action.
    Action,
    Constraint,
}

/// The rows an object covers, by key, in the relation of catalog id `rel`.
#[derive(Clone, Debug)]
struct Covers {
    obj: Obj,
    id: u64,
    rel: u64,
    rel_name: String,
    /// The constructors it ranges over by name, `None` for every one.
    scope: Option<BTreeSet<String>>,
    /// Its text without its scope.
    what: String,
    text: String,
    rows: BTreeSet<Vec<Value>>,
    /// A delete action, and the name of its field.
    action: Option<(String, String)>,
}

/// The rows of one relation: each key's constructor and field names.
type Held = BTreeMap<Vec<Value>, (String, BTreeSet<String>)>;

/// Returns every covering object of the relations of module `app` of `schema`, `names` the
/// names of each declaration of its source, `rows` the rows of `case` that declares it, and the
/// rows of each relation by its catalog id.
fn covers(
    schema: &Schema,
    names: &[Names],
    case: &schemas::Case,
    rows: &Rows,
) -> (Vec<Covers>, BTreeMap<u64, Held>) {
    let ids = schema.ids();
    let mut out = Vec::new();
    let mut all = BTreeMap::new();
    for (r, rel_id) in app_relations(schema) {
        let rel = schema.relation(r);
        let t = elem(schema, r);
        let ty = schema.ty(t);
        let names: Vec<&str> = ty.ctors().iter().map(|c| c.name.as_str()).collect();
        let held: Held = keyed(case, rows, &rel.name)
            .into_iter()
            .map(|(k, c, fs)| (k, (c, fs)))
            .collect();
        let sum = is_sum(schema, t);
        let scope_of = |cs: Option<&[u32]>| -> Option<BTreeSet<String>> {
            cs.filter(|_| sum)
                .map(|cs| cs.iter().map(|c| names[*c as usize].to_owned()).collect())
        };
        let within = |cs: Option<&[u32]>, needs: &[String]| -> BTreeSet<Vec<Value>> {
            held.iter()
                .filter(|(_, (c, fs))| {
                    cs.is_none_or(|cs| cs.iter().any(|i| names[*i as usize] == c.as_str()))
                        && needs.iter().all(|n| fs.contains(n))
                })
                .map(|(k, _)| k.clone())
                .collect()
        };
        let mut push = |obj, id: Option<u64>, what: String, cs: Option<&[u32]>, rows, action| {
            if let Some(id) = id {
                let scope = scope_of(cs);
                let shown = scope.as_ref().map_or(String::new(), |s| {
                    format!(
                        " in ({})",
                        s.iter().cloned().collect::<Vec<_>>().join(" | ")
                    )
                });
                out.push(Covers {
                    obj,
                    id,
                    rel: rel_id,
                    rel_name: rel.name.clone(),
                    scope,
                    text: format!("{}: {what}{shown}", rel.name),
                    what,
                    rows,
                    action,
                });
            }
        };
        push(
            Obj::Key,
            ids.key_id(r).map(u64::from),
            format!("key ({})", rel.key.join(", ")),
            None,
            within(None, &[]),
            None,
        );
        for (at, x) in rel.indexes.iter().enumerate() {
            let elems: Vec<&str> = (0..x.fields.len()).map(|k| x.text(k)).collect();
            let roots: Vec<String> = elems
                .iter()
                .filter_map(|e| e.split([' ', '.']).next().map(str::to_owned))
                .collect();
            let (obj, verb) = if x.unique {
                (Obj::Unique, "unique")
            } else {
                (Obj::Index, "index")
            };
            push(
                obj,
                ids.index_id(r, at).map(u64::from),
                format!("{verb} ({})", elems.join(", ")),
                x.scope.as_deref(),
                within(x.scope.as_deref(), &roots),
                None,
            );
        }
        for s in &rel.sets {
            push(
                Obj::Set,
                ids.set_id(r, &s.name).map(u64::from),
                format!("set {}", s.name),
                Some(&s.ctors),
                within(Some(&s.ctors), &[]),
                None,
            );
        }
        for (at, d) in rel.defaults.iter().enumerate() {
            let scope = d.ctor.map(|c| vec![c]);
            for (f, id) in ids.clause_default_ids(r, at) {
                let (_, name) = field_name(schema, f);
                push(
                    Obj::Default,
                    Some(u64::from(id)),
                    format!("default {name}"),
                    scope.as_deref(),
                    within(scope.as_deref(), &[name]),
                    None,
                );
            }
        }
        for (at, c) in rel.computed.iter().enumerate() {
            push(
                Obj::Computed,
                ids.computed_id(r, at).map(u64::from),
                format!("computed {}", c.name),
                None,
                within(None, &[]),
                None,
            );
        }
        for (f, id) in ids.type_field_ids(t) {
            let (c, name) = field_name(schema, f);
            let def = match c {
                Some(c) => &ty.ctors()[c as usize].fields[f.pos() as usize],
                None => &ty.header()[f.pos() as usize],
            };
            let shown = schema.show_ty(&def.ty);
            if !shown.starts_with("Key ") && !shown.starts_with("Option (Key ") {
                continue;
            }
            let scope = c.map(|c| vec![c]);
            let acts = action(schema, t, &def.on_delete);
            push(
                Obj::Action,
                Some(u64::from(id)),
                format!("{name} on delete {acts}"),
                scope.as_deref(),
                within(scope.as_deref(), std::slice::from_ref(&name)),
                Some((acts, name)),
            );
        }
        all.insert(rel_id, held);
    }
    out.extend(checks(schema, &all));
    out.extend(constraints(schema, names, &all));
    (out, all)
}

/// Returns each check of the relations of module `app` of `schema` with the rows it covers of
/// `all`, the rows of each relation by its catalog id.
fn checks(schema: &Schema, all: &BTreeMap<u64, Held>) -> Vec<Covers> {
    let mut out = Vec::new();
    for (i, k) in schema.check_defs() {
        let Some(&(_, rel_id)) = app_relations(schema).iter().find(|(r, _)| *r == k.relation)
        else {
            continue;
        };
        let rel = schema.relation(k.relation);
        let names: Vec<String> = schema
            .ty(rel.elem_def)
            .ctors()
            .iter()
            .map(|c| c.name.clone())
            .collect();
        let held = &all[&rel_id];
        let scope: Option<BTreeSet<String>> = k
            .ctors
            .as_ref()
            .filter(|_| is_sum(schema, rel.elem_def))
            .map(|cs| cs.iter().map(|i| names[*i as usize].clone()).collect());
        if let Some(id) = schema.ids().check_id(i) {
            out.push(Covers {
                obj: Obj::Check,
                id: u64::from(id),
                rel: rel_id,
                rel_name: rel.name.clone(),
                rows: ranged(held, scope.as_ref(), &[]),
                what: format!("check {}", k.name),
                text: format!("{}: check {} {scope:?}", rel.name, k.name),
                scope,
                action: None,
            });
        }
    }
    out
}

/// Returns the keys of the rows of `held` of a constructor of `scope`, or of any when `None`,
/// holding every field of `needs`.
fn ranged(held: &Held, scope: Option<&BTreeSet<String>>, needs: &[String]) -> BTreeSet<Vec<Value>> {
    held.iter()
        .filter(|(_, (c, fs))| {
            scope.is_none_or(|s| s.contains(c)) && needs.iter().all(|n| fs.contains(n))
        })
        .map(|(key, _)| key.clone())
        .collect()
}

/// Returns each constraint of module `app` of `schema` with the rows of `all` it ranges over:
/// those of the relation and constructors the names of its query resolve to, `names` the names
/// of each declaration, holding every field they resolve to.
fn constraints(schema: &Schema, names: &[Names], all: &BTreeMap<u64, Held>) -> Vec<Covers> {
    let ids = schema.ids();
    let ctors: BTreeMap<u64, String> = ids
        .ctor_ids()
        .filter_map(|(c, id)| {
            let ctor = schema.ty(c.def()).ctors().get(c.index() as usize)?;
            Some((u64::from(id), ctor.name.clone()))
        })
        .collect();
    let fields: BTreeMap<u64, String> = ids
        .field_ids()
        .map(|(f, id)| (u64::from(id), field_name(schema, f).1))
        .collect();
    let rels: BTreeMap<u64, String> = app_relations(schema)
        .into_iter()
        .map(|(r, id)| (id, schema.relation(r).name.clone()))
        .collect();
    let app = schema.module_named("app");
    let mut out = Vec::new();
    for (i, k) in schema.constraint_defs() {
        let (Some(id), Some(n)) = (ids.constraint_id(i), names.get(k.file.0 as usize)) else {
            continue;
        };
        if Some(k.module) != app {
            continue;
        }
        let (mut rel, mut scope, mut needs) = (None, BTreeSet::new(), Vec::new());
        for (_, at, _) in n.ids_within(k.query.span.range()) {
            for &(space, x) in at {
                match space {
                    Space::Shared if rels.contains_key(&x) => rel = Some(x),
                    Space::Shared => scope.extend(ctors.get(&x).cloned()),
                    Space::Field => needs.extend(fields.get(&x).cloned()),
                    _ => {}
                }
            }
        }
        let Some(rel) = rel else {
            continue;
        };
        let scope = (!scope.is_empty()).then_some(scope);
        out.push(Covers {
            obj: Obj::Constraint,
            id: u64::from(id),
            rel,
            rel_name: rels[&rel].clone(),
            rows: ranged(&all[&rel], scope.as_ref(), &needs),
            what: format!("constraint {}", k.name),
            text: format!("{}: constraint {} {scope:?}", rels[&rel], k.name),
            scope,
            action: None,
        });
    }
    out
}

/// What the search reached.
#[derive(Default)]
struct Reached {
    counts: BTreeMap<&'static str, usize>,
    judged: BTreeSet<Obj>,
}

/// The constructors of the relation an object's rows go to, by its relation's name and the
/// constructors it ranged over; `None` where the model does not follow.
type Image<'a> = &'a dyn Fn(&str, &BTreeSet<String>) -> Option<(String, BTreeSet<String>)>;

/// The name a field takes, by its relation's name, its constructor and its name.
type FieldImage<'a> =
    &'a dyn Fn(&str, &BTreeSet<String>, &str) -> Result<BTreeMap<String, String>, String>;

/// The name a delete action's field takes after a migration: one name, or one on each
/// constructor when the constructors it reaches name it apart.
enum Named {
    One(String),
    By(BTreeMap<String, String>),
}

impl Named {
    fn of(by: BTreeMap<String, String>) -> Named {
        let names: BTreeSet<&String> = by.values().collect();
        match names.into_iter().collect::<Vec<_>>().as_slice() {
            [one] => Named::One((*one).clone()),
            _ => Named::By(by),
        }
    }

    /// Returns the name on constructor `c`.
    fn on(&self, c: &str) -> Option<&String> {
        match self {
            Named::One(n) => Some(n),
            Named::By(by) => by.get(c),
        }
    }

    /// Returns whether `name` is the field's name on every constructor of `scope`.
    fn names(&self, name: &str, scope: Option<&BTreeSet<String>>) -> bool {
        match (self, scope) {
            (Named::One(n), _) => n == name,
            (Named::By(by), Some(s)) => s.iter().all(|c| by.get(c).is_some_and(|n| n == name)),
            (Named::By(_), None) => false,
        }
    }
}

/// Returns the count naming a clause of `obj` re-scoped.
fn re_scoped(obj: Obj) -> &'static str {
    match obj {
        Obj::Key => "a key re-scoped",
        Obj::Unique => "a unique re-scoped",
        Obj::Index => "an index re-scoped",
        Obj::Set => "a set re-scoped",
        Obj::Check => "a check re-scoped",
        Obj::Default => "a default re-scoped",
        Obj::Computed => "a computed field re-scoped",
        Obj::Action => "a delete action re-scoped",
        Obj::Constraint => "a constraint re-scoped",
    }
}

/// Returns each object of `was` that `judged` picks whose rows the objects of `now` do not cover
/// as the image of those rows, or whose constructors they do not range over as their `image`,
/// each side with the rows of each relation by its catalog id. A delete action's field is found
/// on the `now` side by its id, else by the name `field` gives it; one `field` fails to name
/// is uncovered.
fn uncovered(
    (was, rows_was): &Covered,
    (now, rows_now): &Covered,
    judged: impl Fn(&Covers) -> bool,
    (image, field): (Image<'_>, FieldImage<'_>),
    reached: &mut Reached,
) -> Vec<String> {
    let old: BTreeSet<(Obj, u64)> = was.iter().map(|c| (c.obj, c.id)).collect();
    let acts = |c: &Covers| c.action.as_ref().map(|x| x.0.clone());
    let mut out = Vec::new();
    for b in was.iter().filter(|b| judged(b)) {
        let empty = Held::new();
        let after = rows_now.get(&b.rel).unwrap_or(&empty);
        let image_name: Option<Named> = match &b.action {
            None => None,
            Some((_, name)) => {
                let by_id = now
                    .iter()
                    .find(|a| a.obj == Obj::Action && a.id == b.id)
                    .and_then(|a| a.action.as_ref().map(|x| x.1.clone()));
                match (by_id, &b.scope) {
                    (Some(now_name), _) => Some(Named::One(now_name)),
                    (None, None) => Some(Named::One(name.clone())),
                    (None, Some(s)) => match field(&b.rel_name, s, name) {
                        Ok(image) => Some(Named::of(image)),
                        Err(e) => {
                            out.push(format!("`{}` has no field image: {e}", b.text));
                            continue;
                        }
                    },
                }
            }
        };
        let holds = |k: &Vec<Value>| {
            after.get(k).is_some_and(|(c, fs)| {
                image_name
                    .as_ref()
                    .is_none_or(|n| n.on(c).is_some_and(|name| fs.contains(name)))
            })
        };
        let rows: BTreeSet<Vec<Value>> = b.rows.iter().filter(|k| holds(k)).cloned().collect();
        reached.judged.insert(b.obj);
        *reached.counts.entry("a clause judged").or_default() += 1;
        let before = &rows_was[&b.rel];
        if rows.iter().any(|k| before[k].0 != after[k].0) {
            *reached
                .counts
                .entry("rows that change constructor")
                .or_default() += 1;
        }
        let same = |a: &&Covers| a.rel == b.rel && a.obj == b.obj;
        let mapped = b
            .scope
            .as_ref()
            .and_then(|s| image(&b.rel_name, s).filter(|(_, to)| to != s));
        if let Some((_, to)) = mapped {
            *reached.counts.entry("a clause re-scoped").or_default() += 1;
            *reached.counts.entry(re_scoped(b.obj)).or_default() += 1;
            let named = |a: &Covers| match (&a.action, &image_name) {
                (Some(x), Some(y)) => y.names(&x.1, a.scope.as_ref()),
                _ => a.what == b.what,
            };
            let parts: Vec<&Covers> = now
                .iter()
                .filter(|a| {
                    let new = !old.contains(&(a.obj, a.id)) && named(a);
                    let onto = a.scope.as_ref().is_none_or(|s| !s.is_disjoint(&to));
                    same(a) && (a.id == b.id || (new && onto))
                })
                .collect();
            let scope: BTreeSet<String> = parts
                .iter()
                .flat_map(|a| a.scope.iter().flatten().cloned())
                .collect();
            let held: BTreeSet<&Vec<Value>> = parts.iter().flat_map(|a| &a.rows).collect();
            let wider = matches!(b.obj, Obj::Index | Obj::Check);
            let covered = if wider {
                rows.iter().all(|k| held.contains(k))
            } else {
                held.len() == rows.len() && rows.iter().all(|k| held.contains(k))
            };
            let one = b.obj != Obj::Unique || parts.len() <= 1;
            let acting = parts.iter().all(|a| acts(a) == acts(b));
            if scope != to || !covered || !one || !acting {
                let texts: Vec<&str> = parts.iter().map(|a| a.text.as_str()).collect();
                out.push(format!(
                    "`{}` re-scopes to {texts:?}, not over {to:?} and the {} rows of its image",
                    b.text,
                    rows.len()
                ));
            }
            continue;
        }
        let kept = now
            .iter()
            .find(|a| same(a) && a.id == b.id && b.obj != Obj::Action);
        match kept {
            Some(a) if a.rows != rows => out.push(format!(
                "`{}` covers {} rows as `{}`, not the {} of its image",
                b.text,
                a.rows.len(),
                a.text,
                rows.len()
            )),
            Some(_) => {}
            None if b.obj == Obj::Action => {
                *reached.counts.entry("a delete action judged").or_default() += 1;
                let named = |c: &Covers| c.action.as_ref().map(|x| x.1.clone());
                let parts: Vec<&Covers> = now
                    .iter()
                    .filter(|a| {
                        let field = a.id == b.id
                            || match (named(a), &image_name) {
                                (Some(x), Some(y)) => y.names(&x, a.scope.as_ref()),
                                (x, y) => x.is_none() && y.is_none(),
                            };
                        let overlaps = if rows.is_empty() {
                            b.scope.is_none() || a.scope == b.scope
                        } else {
                            !a.rows.is_disjoint(&rows)
                        };
                        same(a) && field && overlaps
                    })
                    .collect();
                let held: BTreeSet<&Vec<Value>> = parts.iter().flat_map(|a| &a.rows).collect();
                let acting = parts.iter().all(|a| acts(a) == acts(b));
                if !rows.iter().all(|k| held.contains(k)) || !acting || parts.is_empty() {
                    let texts: Vec<&str> = parts.iter().map(|a| a.text.as_str()).collect();
                    out.push(format!(
                        "no field takes `{}` over the {} rows of its image, its fields after: {texts:?}",
                        b.text,
                        rows.len()
                    ));
                }
            }
            None => {
                *reached.counts.entry("a clause renewed").or_default() += 1;
                let made = now
                    .iter()
                    .any(|a| same(&a) && !old.contains(&(a.obj, a.id)) && a.rows == rows);
                if !made {
                    out.push(format!(
                        "no clause renews `{}` over the {} rows of its image",
                        b.text,
                        rows.len()
                    ));
                }
            }
        }
    }
    out
}

/// The kinds the search schedules first in a migration: those that rewrite clauses of the
/// relations they keep, each replacing constructors three times.
const DRAWN: [Kind; 22] = [
    Kind::Split,
    Kind::Merge,
    Kind::Nest,
    Kind::Unnest,
    Kind::RenameRelation,
    Kind::RenameSet,
    Kind::Split,
    Kind::Merge,
    Kind::Nest,
    Kind::Unnest,
    Kind::RenameConstructor,
    Kind::RenameField,
    Kind::MoveFieldUp,
    Kind::MoveFieldDown,
    Kind::Split,
    Kind::Merge,
    Kind::Nest,
    Kind::Unnest,
    Kind::AddConstructor,
    Kind::Enable,
    Kind::Deprecate,
    Kind::RemoveConstructor,
];

/// How many migrations of a kind of [`MOVING`] a case draws to take one that moves a row to
/// another constructor.
const CANDIDATES: u64 = 8;

/// Returns whether migration `m` over `case` takes a row of `rows` to another constructor.
fn moves(case: &schemas::Case, rows: &Rows, m: &Migration) -> bool {
    let Ok(after) = m.rows(rows) else {
        return false;
    };
    case.relations().iter().any(|(r, _)| {
        let now: BTreeMap<Vec<Value>, String> = keyed(case, &after, r)
            .into_iter()
            .map(|(k, c, _)| (k, c))
            .collect();
        keyed(case, rows, r)
            .into_iter()
            .any(|(k, c, _)| now.get(&k).is_some_and(|d| *d != c))
    })
}

/// The kinds a case draws past its migrations while no row has changed constructor.
const MOVING: [Kind; 4] = [
    Kind::RenameConstructor,
    Kind::Split,
    Kind::Merge,
    Kind::Unnest,
];

/// The counts the test asserts reached. A case draws past its migrations until it reaches them.
const FLOORS: [&str; 6] = [
    "a clause judged",
    "rows that change constructor",
    "a clause re-scoped",
    "a revert judged",
    "a one-product split",
    "a merge with an empty side",
];

/// The clauses of a constructor a split, merge, nest or unnest replaces that the generator
/// draws with every switch off, each counted re-scoped under the profile `deep`.
const RE_SCOPED: [Obj; 6] = [
    Obj::Unique,
    Obj::Index,
    Obj::Set,
    Obj::Check,
    Obj::Action,
    Obj::Constraint,
];

/// Count in `seen` the shapes of the splits and merges of `m` over `rows`.
fn shapes(m: &Migration, rows: &Rows, seen: &mut Reached) {
    for (kind, rel, sources, products) in m.replaced() {
        let held = |c: &String| {
            rows.of(&rel)
                .is_some_and(|rs| rs.iter().any(|(d, _)| d == c))
        };
        if kind == Kind::Split && products.len() == 1 {
            *seen.counts.entry("a one-product split").or_default() += 1;
        }
        if matches!(kind, Kind::Merge | Kind::Nest) && !sources.iter().all(held) {
            *seen.counts.entry("a merge with an empty side").or_default() += 1;
        }
    }
}

/// The kinds that replace constructors.
const STRUCTURAL: [Kind; 4] = [Kind::Split, Kind::Merge, Kind::Nest, Kind::Unnest];

/// The kinds that add a clause, constraint or default; a migration adding one before replacing
/// constructors is applied as two.
const ADDED: [Kind; 4] = [
    Kind::AddUnique,
    Kind::AddCheck,
    Kind::AddDefault,
    Kind::AddConstraint,
];

/// Returns why the checker refuses migration `m` over `source` for a split, merge, nest or
/// unnest whose preconditions read no data, when `m` without it is admitted.
fn refused(source: &Source, m: &Migration) -> Option<String> {
    let admits = |text: &str| {
        parse_migration(text, Options::default())
            .ok()
            .is_ok_and(|f| source.migrate(&f, text).is_ok())
    };
    let (_, o) = m.ops().iter().enumerate().find(|(j, o)| {
        STRUCTURAL.contains(&o.kind()) && !o.reads() && !o.broken() && admits(&m.without(*j).text())
    })?;
    let why = parse_migration(&m.text(), Options::default())
        .ok()
        .ok()
        .and_then(|f| source.migrate(&f, &m.text()).err())
        .map(|f| match f {
            adtb_lang::evolve::Failed::Refused(r) => {
                let ops = r.operators.iter().map(|d| d.message.clone());
                let recheck = r.recheck.iter().map(|d| d.diag.message.clone());
                ops.chain(recheck).collect::<Vec<_>>().join("; ")
            }
            adtb_lang::evolve::Failed::Broken(d) => format!("the model fails: {}", d.message),
        });
    Some(format!(
        "the checker refuses `{}`, which the model admits: {why:?}",
        o.text().lines().next().unwrap_or_default()
    ))
}

/// The objects a migration's search compares, with the rows of each relation by catalog id.
type Covered = (Vec<Covers>, BTreeMap<u64, Held>);

/// Revert migration `m`, the last of `walk`, and judge the objects of `mid`, those after it,
/// against those the revert leaves, each kept one of `was`, those before it, over `rows`. Fails
/// with the revert's text and why.
fn reverted(
    walk: &mut Walk,
    m: &Migration,
    (was, mid): (&Covered, &Covered),
    rows: &Rows,
    seen: &mut Reached,
) -> Result<(), (String, String)> {
    let revert = walk.revert();
    let fail = |why: String| (revert.clone(), why);
    let restored = match walk.apply(&revert) {
        Ok(Some(s)) => s,
        Ok(None) => return Err(fail("the revert is refused".into())),
        Err(e) => return Err(fail(e)),
    };
    let names = walk
        .source
        .names()
        .map_err(|_| fail("the names do not resolve".into()))?;
    let now = covers(&restored, &names, &walk.case, rows);
    let kept = |c: &Covers| was.0.iter().any(|w| (w.obj, w.id) == (c.obj, c.id));
    let back = |rel: &str, cs: &BTreeSet<String>| {
        let from = was.0.iter().find(|w| {
            w.scope
                .as_ref()
                .is_some_and(|s| m.ctors(&w.rel_name, s).ok() == Some((rel.to_owned(), cs.clone())))
        })?;
        Some((from.rel_name.clone(), from.scope.clone()?))
    };
    let field_back =
        |rel: &str, cs: &BTreeSet<String>, f: &str| -> Result<BTreeMap<String, String>, String> {
            let mut found = BTreeSet::new();
            for w in &was.0 {
                let (Some((_, name)), Some(scope)) = (&w.action, &w.scope) else {
                    continue;
                };
                let (r, image) = m.ctors(&w.rel_name, scope)?;
                if r == rel && !image.is_disjoint(cs) && m.field(&w.rel_name, scope, name)? == f {
                    found.insert(name.clone());
                }
            }
            let mut found = found.into_iter();
            match (found.next(), found.next()) {
                (Some(name), None) => Ok(cs.iter().map(|c| (c.clone(), name.clone())).collect()),
                (None, _) => Err(format!(
                    "no field before `{}` is `{f}` of `{rel}`",
                    m.text()
                )),
                (Some(a), Some(b)) => Err(format!("`{f}` of `{rel}` was both `{a}` and `{b}`")),
            }
        };
    let lost = uncovered(mid, &now, kept, (&back, &field_back), seen);
    if !lost.is_empty() {
        return Err(fail(lost.join("\n")));
    }
    *seen.counts.entry("a revert judged").or_default() += 1;
    Ok(())
}

/// Run the coverage search over the case of `seed`.
fn search(seed: u64, at: &mut Option<Failed>, reached: &Mutex<Reached>) -> Option<Failed> {
    let failed = |k, log: &[String], text: &str, finding: String| {
        Some(Failed {
            seed,
            k,
            log: log.to_vec(),
            text: text.to_owned(),
            finding,
        })
    };
    let mut walk = match Walk::new(seed) {
        Ok(w) => w,
        Err(e) => return failed(0, &[], "", e),
    };
    let (mut fx, mut rows) = match filled(&walk.case, &Rows::default()) {
        Ok(x) => x,
        Err(e) => return failed(0, &walk.log, "", e),
    };
    let mut seen = Reached::default();
    for k in 0..migrations() * STRETCH {
        if k >= migrations() && FLOORS.iter().all(|f| seen.counts.contains_key(f)) {
            break;
        }
        let moving = k >= migrations() && !seen.counts.contains_key("rows that change constructor");
        let kind = scheduled(if moving { &MOVING } else { &DRAWN }, seed, k);
        let drawn = if moving {
            let can: Vec<Migration> = walk.draws(k, kind, CANDIDATES).collect();
            can.iter()
                .find(|m| moves(&walk.case, &rows, m))
                .or(can.first())
                .cloned()
        } else {
            walk.draw(k, kind)
        };
        let Some(m) = drawn else {
            continue;
        };
        let text = m.text();
        *at = failed(k, &walk.log, &text, String::new());
        let emptied = m.emptied();
        if !emptied.is_empty() {
            for s in walk.case.emptying(&emptied).stmts() {
                if let Err(e) = write(&mut fx, s.text()) {
                    return failed(k, &walk.log, &text, e);
                }
            }
            rows = match held(&fx, &walk.case) {
                Ok(r) => r,
                Err(f) => return failed(k, &walk.log, &text, format!("{f:?}")),
            };
            let left = |t: &String| {
                let (rel, ctor) = t.split_once('.').unwrap_or((t, ""));
                rows.of(rel)
                    .is_some_and(|rs| rs.iter().any(|(c, _)| ctor.is_empty() || c == ctor))
            };
            if emptied.iter().any(left) {
                continue;
            }
        }
        let setup = m
            .ops()
            .iter()
            .position(|o| STRUCTURAL.contains(&o.kind()))
            .filter(|j| m.ops()[..*j].iter().any(|o| ADDED.contains(&o.kind())));
        let (m, staged) = match setup {
            Some(j) => {
                let (first, rest) = m.split_at(j);
                match walk.apply(&first.text()) {
                    Ok(Some(_)) => {}
                    Ok(None) => continue,
                    Err(e) => return failed(k, &walk.log, &first.text(), e),
                }
                let Ok(set) = first.rows(&rows) else {
                    continue;
                };
                rows = set;
                (rest, true)
            }
            None => (m, false),
        };
        let text = m.text();
        let (Ok(pre_compiled), Ok(pre_names)) = (walk.source.check(), walk.source.names()) else {
            return failed(k, &walk.log, &text, "the source does not check".into());
        };
        let pre_schema = pre_compiled.schema();
        let log = walk.log.clone();
        let mid_schema = match walk.apply(&text) {
            Ok(Some(s)) => s,
            Ok(None) => match refused(&walk.source, &m) {
                Some(f) => return failed(k, &log, &text, f),
                None if staged => {
                    let revert = walk.revert();
                    match walk.apply(&revert) {
                        Ok(Some(_)) => continue,
                        Ok(None) => {
                            return failed(k, &log, &revert, "the revert is refused".into());
                        }
                        Err(e) => return failed(k, &log, &revert, e),
                    }
                }
                None => continue,
            },
            Err(e) => return failed(k, &log, &text, e),
        };
        let migrated = m.applied(seed.wrapping_mul(1009) ^ k, STATEMENTS);
        let Ok(mid_rows) = m.rows(&rows) else {
            walk.case = migrated;
            (fx, rows) = match filled(&walk.case, &Rows::default()) {
                Ok(x) => x,
                Err(e) => return failed(k, &walk.log, &text, e),
            };
            continue;
        };
        let Ok(mid_names) = walk.source.names() else {
            return failed(k, &log, &text, "the names do not resolve".into());
        };
        shapes(&m, &rows, &mut seen);
        let was = covers(pre_schema, &pre_names, &walk.case, &rows);
        let mid = covers(&mid_schema, &mid_names, &migrated, &mid_rows);
        let judged = !m.ops().iter().any(|o| UNJUDGED.contains(&o.kind()));
        let image = |rel: &str, cs: &BTreeSet<String>| m.ctors(rel, cs).ok();
        let field = |rel: &str, cs: &BTreeSet<String>, f: &str| m.fields(rel, cs, f);
        if judged {
            let lost = uncovered(&was, &mid, |_| true, (&image, &field), &mut seen);
            if !lost.is_empty() {
                return failed(k, &log, &text, lost.join("\n"));
            }
        }
        if judged && !staged && m.exact() && k % 2 == 0 {
            if let Err((text, finding)) = reverted(&mut walk, &m, (&was, &mid), &rows, &mut seen) {
                return failed(k, &walk.log, &text, finding);
            }
            continue;
        }
        walk.case = migrated;
        (fx, rows) = match filled(&walk.case, &mid_rows) {
            Ok(x) => x,
            Err(e) => return failed(k, &walk.log, &text, e),
        };
    }
    let mut r = reached
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (what, n) in seen.counts {
        *r.counts.entry(what).or_default() += n;
    }
    r.judged.extend(seen.judged);
    None
}

#[test]
fn a_rewritten_clause_covers_the_image_of_the_rows_it_covered() {
    let reached = Mutex::new(Reached::default());
    let failures = each_seed(|seed, at| search(seed, at, &reached));
    report("clause coverage", &failures);
    let reached = reached
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let n = |what: &str| reached.counts.get(what).copied().unwrap_or(0);
    for f in FLOORS {
        profile::assert_reached(f, n(f), 1);
    }
    if profile::every_case() {
        for o in RE_SCOPED {
            profile::assert_reached(re_scoped(o), n(re_scoped(o)), 1);
        }
        let all = [
            Obj::Key,
            Obj::Unique,
            Obj::Index,
            Obj::Set,
            Obj::Check,
            Obj::Default,
            Obj::Computed,
            Obj::Action,
            Obj::Constraint,
        ];
        let missed: Vec<Obj> = all
            .into_iter()
            .filter(|o| !reached.judged.contains(o))
            .collect();
        assert_eq!(missed, Vec::<Obj>::new(), "objects never judged");
    }
}
