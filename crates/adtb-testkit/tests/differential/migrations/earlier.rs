//! The defaults a revert of an earlier migration leaves after later ones, or the refusals it
//! gives, predicted from the catalog the migration found, the one it left and the one before the
//! revert.

use std::collections::{BTreeMap, BTreeSet};

use adtb_reference::Value;

use adtb_testkit::schemas::{Kind, Reach};

use super::super::Got;
use super::reasons;

/// A row of `catalog::defaults`.
#[derive(Clone, Debug)]
struct DefaultRow {
    id: Value,
    relation: Value,
    field: Value,
    text: String,
}

/// A row of `catalog::fields`: its name, and its constructor when it is a constructor's.
#[derive(Clone, Debug)]
struct FieldRow {
    name: String,
    ctor: Option<Value>,
}

/// An object of a catalog relation with a name: its name, and the object it belongs to, by its
/// catalog relation and id.
#[derive(Clone, Debug)]
struct Object {
    name: String,
    owner: Option<(&'static str, Value)>,
}

/// A catalog relation [`Object`] reads.
struct Objects {
    relation: &'static str,
    /// The column naming its rows.
    name: &'static str,
    /// The columns naming the object a row belongs to, each with that object's relation.
    owners: &'static [(&'static str, &'static str)],
    /// The word a refusal names such an object by.
    word: &'static str,
}

const OBJECTS: [Objects; 6] = [
    Objects {
        relation: "types",
        name: "name",
        owners: &[],
        word: "type",
    },
    Objects {
        relation: "constructors",
        name: "name",
        owners: &[("sum", "types")],
        word: "constructor",
    },
    Objects {
        relation: "fields",
        name: "name",
        owners: &[("ctor", "constructors"), ("sum", "types")],
        word: "field",
    },
    Objects {
        relation: "relations",
        name: "name",
        owners: &[("sum", "types")],
        word: "relation",
    },
    Objects {
        relation: "named_sets",
        name: "name",
        owners: &[("relation", "relations")],
        word: "set",
    },
    Objects {
        relation: "indexes",
        name: "clause",
        owners: &[("relation", "relations")],
        word: "index",
    },
];

/// The defaults, fields and constructors of a catalog no migration dropped, and its named
/// objects by relation and id.
pub(super) struct Catalog {
    defaults: Vec<DefaultRow>,
    fields: BTreeMap<Value, FieldRow>,
    ctors: BTreeMap<Value, String>,
    objects: BTreeMap<&'static str, BTreeMap<Value, Object>>,
}

/// A field a default fills after a revert: one held before it, by its id, or one the revert
/// puts back, by its constructor's name and its own.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum At {
    Held(Value),
    Back(String, String),
}

/// What a revert of an earlier migration does to the defaults.
#[derive(Debug)]
pub(super) struct Want {
    /// The defaults it leaves when it commits, none when it is refused.
    pub(super) commits: Option<BTreeSet<(At, String)>>,
    /// The refusals it may give, each a pattern of [`reasons::reverted`].
    pub(super) refusals: Vec<String>,
}

fn column<'a>(v: &'a Value, name: &str) -> Option<&'a Value> {
    match v {
        Value::Ctor(_, fs) | Value::Record(fs) => fs.iter().find(|(n, _)| n == name).map(|f| &f.1),
        _ => None,
    }
}

fn text(v: &Value, name: &str) -> Result<String, String> {
    match column(v, name) {
        Some(Value::Text(s)) => Ok(s.clone()),
        other => Err(format!("`{name}` of {v:?} is {other:?}, not a text")),
    }
}

fn held(v: &Value, name: &str) -> Result<Value, String> {
    column(v, name)
        .cloned()
        .ok_or_else(|| format!("{v:?} has no `{name}`"))
}

/// Returns whether column `dropped` of `v` is `None`.
fn kept(v: &Value) -> Result<bool, String> {
    match column(v, "dropped") {
        Some(Value::Ctor(_, fs)) if fs.is_empty() => Ok(true),
        Some(Value::Ctor(_, fs)) if fs.len() == 1 && fs[0].0 == "value" => Ok(false),
        other => Err(format!("`dropped` of {v:?} is {other:?}, not an option")),
    }
}

/// Returns the rows of `catalog::{relation}` of `cat` no migration dropped.
fn rows(cat: &[Got], relation: &str) -> Result<Vec<Value>, String> {
    let at = super::CATALOG
        .iter()
        .position(|r| *r == relation)
        .ok_or_else(|| format!("the catalog reads no `{relation}`"))?;
    let Some(Got::Rows(rows)) = cat.get(at) else {
        return Err(format!("`catalog::{relation}` reads as {:?}", cat.get(at)));
    };
    let mut out = Vec::new();
    for r in rows {
        if kept(r)? {
            out.push(r.clone());
        }
    }
    Ok(out)
}

impl Catalog {
    /// Returns the defaults, fields, constructors and named objects of catalog relations `cat`,
    /// read as [`super::catalog`] reads them.
    pub(super) fn of(cat: &[Got]) -> Result<Catalog, String> {
        let mut objects = BTreeMap::new();
        for o in OBJECTS {
            let mut held_rows = BTreeMap::new();
            for r in rows(cat, o.relation)? {
                let owner = o
                    .owners
                    .iter()
                    .find_map(|(column_name, of)| Some((*of, column(&r, column_name)?.clone())));
                let object = Object {
                    name: text(&r, o.name)?,
                    owner,
                };
                held_rows.insert(held(&r, "id")?, object);
            }
            objects.insert(o.relation, held_rows);
        }
        let mut defaults = Vec::new();
        for d in rows(cat, "defaults")? {
            defaults.push(DefaultRow {
                id: held(&d, "id")?,
                relation: held(&d, "relation")?,
                field: held(&d, "field")?,
                text: text(&d, "text")?,
            });
        }
        let mut fields = BTreeMap::new();
        for f in rows(cat, "fields")? {
            let row = FieldRow {
                name: text(&f, "name")?,
                ctor: column(&f, "ctor").cloned(),
            };
            fields.insert(held(&f, "id")?, row);
        }
        let mut ctors = BTreeMap::new();
        for c in rows(cat, "constructors")? {
            ctors.insert(held(&c, "id")?, text(&c, "name")?);
        }
        Ok(Catalog {
            defaults,
            fields,
            ctors,
            objects,
        })
    }

    fn holds_default(&self, id: &Value) -> bool {
        self.defaults.iter().any(|d| d.id == *id)
    }
}

/// Returns field `f` of catalog `of` by its constructor's name, as catalog `now` names that
/// constructor when it holds it, and its own name; the empty name for a header field.
///
/// Fail when `of` holds no field `f`, or no constructor of a field of a constructor.
fn named(of: &Catalog, now: &Catalog, f: &Value) -> Result<At, String> {
    let field = of
        .fields
        .get(f)
        .ok_or_else(|| format!("the catalog holds no field {f:?} a default fills"))?;
    let ctor = match &field.ctor {
        Some(c) => now
            .ctors
            .get(c)
            .or_else(|| of.ctors.get(c))
            .cloned()
            .ok_or_else(|| format!("the catalog holds no constructor {c:?} of field {f:?}"))?,
        None => String::new(),
    };
    Ok(At::Back(ctor, field.name.clone()))
}

/// Returns what a revert of a migration does to the defaults of catalog `now`, the catalog
/// before the revert, given the catalogs `found`, before the migration, and `left`, after it,
/// the texts of the migrations `later` than it, and whether it drops a field it states no fill
/// for.
///
/// A default the migration added goes. A default the migration dropped comes back to each field
/// it filled, refused when a later migration dropped one of them or one of them has another
/// default. A field the migration dropped comes back with the default it had when it was
/// dropped. A revert may be refused besides for an object its inverse names that a later
/// migration dropped or renamed, for a name a later migration took, for a missing fill, for a
/// field with no default a later migration added to a type it puts a field or constructor back
/// in, and for a later migration naming an object the migration added.
///
/// Fail when a default fills a field its catalog does not hold.
pub(super) fn predict(
    now: &Catalog,
    found: &Catalog,
    left: &Catalog,
    later: &[String],
    unfilled: bool,
) -> Result<Want, String> {
    let back = |f: &Value| !left.fields.contains_key(f);
    let dropped: Vec<&DefaultRow> = found
        .defaults
        .iter()
        .filter(|d| !left.holds_default(&d.id))
        .collect();
    let stays: Vec<&DefaultRow> = now
        .defaults
        .iter()
        .filter(|d| found.holds_default(&d.id) || !left.holds_default(&d.id))
        .collect();
    let mut live: BTreeSet<(At, String)> = stays
        .iter()
        .map(|d| (At::Held(d.field.clone()), d.text.clone()))
        .collect();
    let (moved, own): (Vec<&DefaultRow>, Vec<&DefaultRow>) =
        dropped.iter().partition(|d| back(&d.field));
    let mut refusals = refusals(now, found, left, &dropped, later, unfilled);
    let mut refused = false;
    if own.iter().any(|d| !now.fields.contains_key(&d.field)) {
        refusals.insert(reasons::FILLS_NO_FIELD.to_owned());
        refused = true;
    }
    if own.iter().any(|d| stays.iter().any(|x| x.field == d.field)) {
        refusals.insert(reasons::ANOTHER_DEFAULT.to_owned());
        refused = true;
    }
    for d in &own {
        live.insert((At::Held(d.field.clone()), d.text.clone()));
    }
    for d in &moved {
        live.insert((named(found, now, &d.field)?, d.text.clone()));
    }
    Ok(Want {
        commits: (!refused).then_some(live),
        refusals: refusals.into_iter().collect(),
    })
}

/// Returns the objects the inverse of a migration names, given the catalogs `found` before it
/// and `left` after it and the defaults `dropped` it dropped, each by its catalog relation and
/// id: what it added, the objects owning what it dropped, and the fields and relations of the
/// defaults it dropped.
fn inverse_names(
    found: &Catalog,
    left: &Catalog,
    dropped: &[&DefaultRow],
) -> BTreeSet<(&'static str, Value)> {
    let mut out = BTreeSet::new();
    for (relation, held_rows) in &left.objects {
        for id in held_rows.keys() {
            if !found.objects[relation].contains_key(id) {
                out.insert((*relation, id.clone()));
            }
        }
    }
    for (relation, held_rows) in &found.objects {
        for (id, object) in held_rows {
            if !left.objects[relation].contains_key(id) {
                out.extend(object.owner.clone());
            }
        }
    }
    for d in dropped {
        out.insert(("fields", d.field.clone()));
        out.insert(("relations", d.relation.clone()));
    }
    let mut owners: Vec<(&'static str, Value)> = out.iter().cloned().collect();
    while let Some((relation, id)) = owners.pop() {
        let owner = [found, left]
            .iter()
            .find_map(|c| c.objects[relation].get(&id)?.owner.clone());
        if let Some(o) = owner
            && out.insert(o.clone())
        {
            owners.push(o);
        }
    }
    out
}

/// Returns the type owning object `id` of catalog relation `relation`, as the first of `cats`
/// holding each object on the way names it.
fn type_of(cats: &[&Catalog], relation: &'static str, id: &Value) -> Option<Value> {
    let (mut relation, mut id) = (relation, id.clone());
    while relation != "types" {
        let owner = cats
            .iter()
            .find_map(|c| c.objects[relation].get(&id))?
            .owner
            .clone()?;
        (relation, id) = owner;
    }
    Some(id)
}

/// Returns the refusals a revert of a migration may give for its inverse naming objects later
/// migrations changed, as [`predict`] states them; the refusals for its defaults aside.
fn refusals(
    now: &Catalog,
    found: &Catalog,
    left: &Catalog,
    dropped: &[&DefaultRow],
    later: &[String],
    unfilled: bool,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let named_then = |relation: &str, id: &Value| {
        [left, found]
            .iter()
            .find_map(|c| c.objects[relation].get(id))
            .map(|o| o.name.clone())
    };
    for (relation, id) in inverse_names(found, left, dropped) {
        let word = OBJECTS
            .iter()
            .find(|o| o.relation == relation)
            .expect("an inverse names objects of relations `OBJECTS` reads")
            .word;
        match now.objects[relation].get(&id) {
            None if relation == "fields" => {
                out.insert(reasons::FILLS_NO_FIELD.to_owned());
            }
            None => {
                out.insert(reasons::later(word));
            }
            Some(o) if named_then(relation, &id).is_some_and(|n| n != o.name) => {
                out.insert(reasons::RENAMED_APART.to_owned());
            }
            Some(_) => {}
        }
    }
    for (relation, kind) in [
        ("fields", Kind::AddField),
        ("constructors", Kind::AddConstructor),
    ] {
        let taken = found.objects[relation]
            .iter()
            .filter(|(id, _)| !left.objects[relation].contains_key(id))
            .any(|(_, o)| {
                now.objects[relation]
                    .values()
                    .any(|n| n.name == o.name && n.owner == o.owner)
            });
        if taken {
            out.extend(reasons::taken(kind));
        }
    }
    if unfilled {
        out.insert(reasons::NEEDS_FILL.to_owned());
    }
    let put_back: BTreeSet<Value> = ["fields", "constructors"]
        .into_iter()
        .flat_map(|relation| {
            found.objects[relation]
                .keys()
                .filter(move |id| !left.objects[relation].contains_key(id))
                .filter_map(move |id| type_of(&[found], relation, id))
        })
        .collect();
    let unfilled_later = now.objects["fields"].keys().any(|id| {
        !left.objects["fields"].contains_key(id)
            && now.defaults.iter().all(|d| d.field != *id)
            && type_of(&[now], "fields", id).is_some_and(|t| put_back.contains(&t))
    });
    if unfilled_later {
        out.insert(reasons::RETAG_UNFILLED.to_owned());
    }
    let added = left.objects.iter().flat_map(|(relation, held_rows)| {
        held_rows
            .iter()
            .filter(|(id, _)| !found.objects[relation].contains_key(id))
            .map(|(_, o)| o.name.as_str())
    });
    let words = |text: &str| -> BTreeSet<String> {
        text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(str::to_owned)
            .collect()
    };
    let later_words: BTreeSet<String> = later.iter().flat_map(|t| words(t)).collect();
    if added.into_iter().any(|n| later_words.contains(n)) {
        out.extend(reasons::DEPENDS.iter().map(|d| (*d).to_owned()));
    }
    out
}

/// Returns the shapes of renamed fields a revert of a migration reaches, given the catalogs `now`
/// before the revert, `found` before the migration and `left` after it:
/// [`Reach::RevertedBesideRenamed`] when it gives a field the migration dropped back its default
/// while a field of another constructor that had a default of that text is renamed since, and
/// [`Reach::RevertedRenamedFill`] when it gives a default the migration dropped back to a field
/// renamed since.
pub(super) fn renamed(now: &Catalog, found: &Catalog, left: &Catalog) -> Vec<Reach> {
    let ctor = |f: &Value| found.fields.get(f).and_then(|x| x.ctor.clone());
    let renamed = |f: &Value| {
        let was = found.fields.get(f);
        let is = now.fields.get(f);
        was.zip(is).is_some_and(|(w, i)| w.name != i.name)
    };
    let beside = found
        .defaults
        .iter()
        .filter(|d| !left.fields.contains_key(&d.field))
        .any(|d| {
            found.defaults.iter().any(|p| {
                p.relation == d.relation
                    && p.text == d.text
                    && p.id != d.id
                    && now.holds_default(&p.id)
                    && ctor(&p.field) != ctor(&d.field)
                    && renamed(&p.field)
            })
        });
    let fill = found.defaults.iter().any(|d| {
        !left.holds_default(&d.id) && left.fields.contains_key(&d.field) && renamed(&d.field)
    });
    let mut out = Vec::new();
    if beside {
        out.push(Reach::RevertedBesideRenamed);
    }
    if fill {
        out.push(Reach::RevertedRenamedFill);
    }
    out
}

/// Returns the defaults of catalog `after`, a revert's, each by where its field is against the
/// catalog `now` before it.
///
/// Fail when a default fills a field `after` does not hold.
pub(super) fn left(now: &Catalog, after: &Catalog) -> Result<BTreeSet<(At, String)>, String> {
    after
        .defaults
        .iter()
        .map(|d| {
            let at = if now.fields.contains_key(&d.field) {
                At::Held(d.field.clone())
            } else {
                named(after, after, &d.field)?
            };
            Ok((at, d.text.clone()))
        })
        .collect()
}

/// Returns how the defaults `got` a revert left differ from `live`, if they do.
pub(super) fn differs(
    live: &BTreeSet<(At, String)>,
    got: &BTreeSet<(At, String)>,
) -> Option<String> {
    (got != live).then(|| {
        format!(
            "missing {:?}, unstated {:?}",
            live.difference(got).collect::<Vec<_>>(),
            got.difference(live).collect::<Vec<_>>()
        )
    })
}

/// Returns the field path and the fill key a refusal of a revert for want of a fill of a
/// dropped field names: "the inverse of `drop field PATH` needs a fill: `fill KEY = e`".
pub(super) fn unfilled(detail: &str) -> Option<(&str, &str)> {
    let rest = detail.strip_prefix("the inverse of `drop field ")?;
    let (path, rest) = rest.split_once("` needs a fill: `fill ")?;
    let key = rest.strip_suffix(" = e`")?;
    Some((path, key))
}
