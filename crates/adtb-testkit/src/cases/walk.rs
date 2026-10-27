use std::sync::Arc;

use super::world::{Ctor, Field, Index, Rel, T, World, value};
use super::{Case, query};
use crate::Rng;

/// A schema walk: a drawn case, then schema operators applied to its database one migration at
/// a time, each followed by writes.
#[derive(Clone, Debug)]
pub struct Walk {
    pub start: Case,
    pub steps: Vec<Step>,
}

/// One migration of a walk and the writes after it.
#[derive(Clone, Debug)]
pub struct Step {
    /// The migration of module `app` the step applies.
    pub migration: String,
    /// The database once the migration applies, as a case of the schema it leaves, with queries
    /// over that schema.
    pub migrated: Case,
    pub writes: Vec<String>,
    /// The database after the writes, with the queries of `migrated`.
    pub written: Case,
}

impl Walk {
    /// Returns the walk's log: the migration of its first case, then each step's.
    pub fn log(&self) -> Vec<String> {
        let mut out = vec![self.start.migration.clone()];
        out.extend(self.steps.iter().map(|s| s.migration.clone()));
        out
    }
}

/// Returns the walk of `seed`: its drawn case holding `queries` queries, then `steps` steps.
pub fn walk(seed: u64, steps: usize, queries: usize) -> Walk {
    let mut rng = Rng::new(seed);
    let mut world = World::draw(&mut rng);
    world.load(&mut rng);
    let start = cased(&mut rng, &world, queries);
    let mut out = Vec::new();
    for _ in 0..steps {
        let mut ops = Vec::new();
        let mut touched = Vec::new();
        for _ in 0..=rng.below(2) {
            let (r, op) = operator(&mut rng, &mut world, &touched);
            touched.push(r);
            ops.extend(op);
        }
        let migrated = cased(&mut rng, &world, queries);
        let n = 1 + rng.below(4);
        let writes = world.write(&mut rng, n);
        let written = Case {
            load: world.inserts(),
            ..migrated.clone()
        };
        let mut migration = String::from("module app\n");
        for op in ops {
            migration.push_str(&op);
            migration.push('\n');
        }
        out.push(Step {
            migration,
            migrated,
            writes,
            written,
        });
    }
    Walk { start, steps: out }
}

fn cased(rng: &mut Rng, world: &World, queries: usize) -> Case {
    let world = Arc::new(world.clone());
    let qs = (0..queries).map(|_| query(rng, &world)).collect();
    Case::drawn(world.clone(), world.inserts(), qs)
}

/// The field types an operator adds.
const ADDED: [T; 6] = [T::Int, T::Text, T::Bool, T::Shade, T::OptInt, T::Float];

/// Applies a schema operator drawn at random to a relation of `w` not in `touched`, and returns
/// that relation and the operator's lines: an added relation, constructor, field or index, a
/// renamed constructor or field, or a dropped field or index. An operator of a migration names
/// nothing an earlier one of it creates.
fn operator(rng: &mut Rng, w: &mut World, touched: &[usize]) -> (usize, Vec<String>) {
    for _ in 0..16 {
        let r = rng.below(w.rels.len() as u64) as usize;
        if touched.contains(&r) {
            continue;
        }
        let ops = match rng.below(8) {
            0 => break,
            1 => add_constructor(rng, w, r),
            2 => add_field(rng, w, r),
            3 => drop_field(rng, w, r),
            4 => rename_field(rng, w, r),
            5 => rename_constructor(rng, w, r),
            6 => add_index(rng, w, r),
            _ => drop_index(rng, w, r),
        };
        if let Some(ops) = ops {
            return (r, ops);
        }
    }
    let ops = add_relation(rng, w);
    (w.rels.len() - 1, ops)
}

fn add_relation(rng: &mut Rng, w: &mut World) -> Vec<String> {
    let rel = Rel::draw(rng, w.rels.len());
    let ops = vec![
        format!("add {}", rel.type_decl()),
        format!("add {}", rel.relation_decl()),
    ];
    w.rels.push(rel);
    ops
}

/// Returns the constructor fields a drawn constructor or field takes, named freshly.
fn fields(rng: &mut Rng, w: &mut World, n: u64) -> Vec<Field> {
    (0..n)
        .map(|_| Field {
            name: w.fresh("f"),
            ty: rng.pick(&ADDED).clone(),
        })
        .collect()
}

fn add_constructor(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    if !w.rels[r].sum {
        return None;
    }
    let name = w.fresh(&format!("C{r}"));
    let n = 1 + rng.below(2);
    let fs = fields(rng, w, n);
    let decl: Vec<String> = fs
        .iter()
        .map(|f| format!("{}: {}", f.name, f.ty.name()))
        .collect();
    let rel = &mut w.rels[r];
    let ops = vec![
        format!(
            "add constructor {}.{name} {{ {} }}",
            rel.ty,
            decl.join(", ")
        ),
        format!("enable {}.{name}", rel.name),
    ];
    rel.ctors.push(Ctor { name, fields: fs });
    Some(ops)
}

/// Returns the path of field `f` of `rel` in an operator: in constructor `c`, or the header.
fn path(rel: &Rel, c: Option<usize>, f: &str) -> String {
    match c {
        Some(c) => format!("{}.{}.{f}", rel.ty, rel.ctors[c].name),
        None => format!("{}.{f}", rel.ty),
    }
}

/// Returns a constructor of a sum `rel`, or the header, drawn at random.
fn place(rng: &mut Rng, rel: &Rel) -> Option<usize> {
    let n = rel.ctors.len() as u64;
    (rel.sum && rng.below(2) == 0).then(|| rng.below(n) as usize)
}

fn add_field(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let c = place(rng, &w.rels[r]);
    let f = fields(rng, w, 1).remove(0);
    let fill = value(rng, &f.ty, &[]);
    let op = format!(
        "add field {}: {} fill {fill}",
        path(&w.rels[r], c, &f.name),
        f.ty.name()
    );
    for row in w.rows.iter_mut().filter(|row| row.rel == r) {
        if c.is_none_or(|c| row.ctor == c) {
            row.fields.push((f.name.clone(), fill.clone()));
        }
    }
    let rel = &mut w.rels[r];
    match c {
        Some(c) => rel.ctors[c].fields.push(f),
        None => rel.header.push(f),
    }
    Some(vec![op])
}

/// Returns a field of `rel` no index names and no key type holds, drawn at random, with its
/// constructor, or `None` for the header.
fn unindexed(rng: &mut Rng, rel: &Rel) -> Option<(Option<usize>, String)> {
    let c = place(rng, rel);
    let fs = match c {
        Some(c) => &rel.ctors[c].fields,
        None => &rel.header,
    };
    let free: Vec<&Field> = fs
        .iter()
        .filter(|f| !matches!(f.ty, T::Key(_)))
        .filter(|f| !rel.indexes.iter().any(|i| i.field == f.name))
        .collect();
    rng.pick_some(&free).map(|f| (c, f.name.clone()))
}

fn drop_field(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let (c, f) = unindexed(rng, &w.rels[r])?;
    let op = format!("drop field {}", path(&w.rels[r], c, &f));
    for row in w.rows.iter_mut().filter(|row| row.rel == r) {
        row.fields.retain(|(g, _)| *g != f);
    }
    let rel = &mut w.rels[r];
    match c {
        Some(c) => rel.ctors[c].fields.retain(|g| g.name != f),
        None => rel.header.retain(|g| g.name != f),
    }
    Some(vec![op])
}

fn rename_field(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let (c, f) = unindexed(rng, &w.rels[r])?;
    let g = w.fresh("f");
    let op = format!("rename field {} to {g}", path(&w.rels[r], c, &f));
    for row in w.rows.iter_mut().filter(|row| row.rel == r) {
        for (name, _) in row.fields.iter_mut().filter(|(name, _)| *name == f) {
            name.clone_from(&g);
        }
    }
    let rel = &mut w.rels[r];
    let fs = match c {
        Some(c) => &mut rel.ctors[c].fields,
        None => &mut rel.header,
    };
    for field in fs.iter_mut().filter(|field| field.name == f) {
        field.name.clone_from(&g);
    }
    Some(vec![op])
}

fn rename_constructor(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    if !w.rels[r].sum {
        return None;
    }
    let to = w.fresh(&format!("C{r}"));
    let rel = &mut w.rels[r];
    let c = rng.below(rel.ctors.len() as u64) as usize;
    let from = std::mem::replace(&mut rel.ctors[c].name, to.clone());
    Some(vec![format!(
        "rename constructor {}.{from} to {to}",
        rel.ty
    )])
}

fn add_index(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let rel = &mut w.rels[r];
    let ctor = place(rng, rel);
    let fs = match ctor {
        Some(c) => &rel.ctors[c].fields,
        None => &rel.header,
    };
    let names: Vec<&Field> = fs
        .iter()
        .filter(|f| matches!(f.ty, T::Int | T::Text | T::Key(_)))
        .collect();
    let index = Index {
        field: rng.pick_some(&names)?.name.clone(),
        ctor,
    };
    if rel.indexes.contains(&index) {
        return None;
    }
    let op = format!(
        "add index ({}) in {}",
        index.field,
        rel.index_target(&index)
    );
    rel.indexes.push(index);
    Some(vec![op])
}

fn drop_index(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let rel = &mut w.rels[r];
    if rel.indexes.is_empty() {
        return None;
    }
    let k = rng.below(rel.indexes.len() as u64) as usize;
    let op = format!(
        "drop index ({}) in {}",
        rel.indexes[k].field,
        rel.index_target(&rel.indexes[k])
    );
    rel.indexes.remove(k);
    Some(vec![op])
}
