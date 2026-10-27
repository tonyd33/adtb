//! Cases for differential testing, drawn type-directed: a schema of record and sum relations,
//! the writes that load it and well-typed queries over it, each a function of its seed alone.

mod draw;
mod expr;
mod walk;
mod world;

use std::collections::BTreeSet;
use std::sync::Arc;

use self::draw::Gen;
use self::expr::{Comp, E, Render, Tried};
use self::world::{T, World};
use crate::Rng;

pub use self::walk::{Step, Walk, walk};

/// A schema, the writes that load a database of it and queries over that database.
#[derive(Clone, Debug)]
pub struct Case {
    /// The schema file of module `app`.
    pub schema: String,
    /// The migration that adds module `app` with the declarations of `schema`.
    pub migration: String,
    pub load: Vec<String>,
    pub queries: Vec<Query>,
    /// The bound two sums of floats of the case agree within.
    pub tolerance: f64,
    /// The relations the schema declares, when the case is drawn.
    world: Option<Arc<World>>,
}

impl Case {
    fn drawn(world: Arc<World>, load: Vec<String>, queries: Vec<Query>) -> Case {
        let mut schema = String::from("module app\n");
        let mut migration = String::from("module app\nadd module app\n");
        for (verb, d) in world.decls() {
            schema.push_str(&format!("{d}\n"));
            migration.push_str(&format!("{verb} {d}\n"));
        }
        Case {
            schema,
            migration,
            load,
            queries,
            tolerance: world.float_bound(),
            world: Some(world),
        }
    }

    /// Returns the case of a schema, its migration, its load and ad hoc queries over it, whose
    /// floats compare exactly.
    pub fn of_text(
        schema: String,
        migration: String,
        load: Vec<String>,
        queries: Vec<String>,
    ) -> Case {
        Case {
            schema,
            migration,
            load,
            queries: queries.into_iter().map(|t| Query(Form::Text(t))).collect(),
            tolerance: 0.0,
            world: None,
        }
    }

    /// Returns the schema and each write as text, a write on a line.
    pub fn program(&self) -> String {
        let mut out = self.schema.clone();
        for w in &self.load {
            out.push_str(w);
            out.push('\n');
        }
        out
    }

    /// Returns the case holding `queries` in place of its own.
    pub fn with_queries(&self, queries: Vec<Query>) -> Case {
        Case {
            queries,
            ..self.clone()
        }
    }

    /// Returns the cases one step smaller than this one in their database: each without a run
    /// of writes of the load, the longest runs first, then each without one index. A write a
    /// smaller load holds may be refused.
    pub fn smaller(&self) -> Vec<Case> {
        let mut out = Vec::new();
        let mut size = self.load.len().div_ceil(2);
        while size > 0 {
            for start in (0..self.load.len()).step_by(size) {
                let mut c = self.clone();
                c.load.drain(start..(start + size).min(self.load.len()));
                out.push(c);
            }
            size /= 2;
        }
        let Some(world) = &self.world else {
            return out;
        };
        for (r, rel) in world.rels.iter().enumerate() {
            for k in 0..rel.indexes.len() {
                let mut w = world.as_ref().clone();
                w.rels[r].indexes.remove(k);
                out.push(Case::drawn(
                    Arc::new(w),
                    self.load.clone(),
                    self.queries.clone(),
                ));
            }
        }
        out
    }
}

/// A query without a predicate and its three restrictions by that predicate: to the bindings
/// where it is true, where it is false, and where it faults.
#[derive(Clone, Debug)]
pub struct Split {
    pub whole: String,
    pub parts: [String; 3],
}

/// The forms a query's text holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Forms {
    pub fix: bool,
    pub group: bool,
    pub matches: bool,
    /// Two generators of one comprehension, or a comprehension reading a variable bound
    /// outside it.
    pub join: bool,
    pub float: bool,
}

/// A query over the relations of a case.
#[derive(Clone, Debug)]
pub struct Query(Form);

#[derive(Clone, Debug)]
enum Form {
    Drawn(Box<Drawn>),
    Text(String),
}

/// A comprehension over the relations of a schema and a predicate of its bindings.
#[derive(Clone, Debug)]
struct Drawn {
    world: Arc<World>,
    /// The type of the select expression.
    sel: T,
    comp: Comp,
    /// The value grouped and the expression grouped by, when the query groups its bindings.
    group: Option<(E, E)>,
    p: E,
}

impl Query {
    pub fn text(&self) -> String {
        match &self.0 {
            Form::Drawn(d) => d.text(),
            Form::Text(t) => t.clone(),
        }
    }

    /// Returns the query once per constructor its first generator ranges over, that generator
    /// ranging over the constructor's relation alone, when it ranges over a sum relation.
    pub fn by_ctor(&self) -> Option<Vec<String>> {
        let Form::Drawn(d) = &self.0 else {
            return None;
        };
        let b = &d.comp.gens[0];
        if !d.world.rels[b.rel].sum || d.group.is_some() {
            return None;
        }
        let parts = b
            .ctors
            .iter()
            .map(|&c| {
                let mut r = Render::new(&d.world);
                r.ctors.insert(b.var.clone(), vec![c]);
                r.comp(&d.comp)
            })
            .collect();
        Some(parts)
    }

    /// Returns the query split by its predicate, when it does not group its bindings.
    pub fn by_predicate(&self) -> Option<Split> {
        let Form::Drawn(d) = &self.0 else {
            return None;
        };
        if d.group.is_some() {
            return None;
        }
        let parts = d.restrictions().map(|c| Render::new(&d.world).comp(&c));
        Some(Split {
            whole: d.text(),
            parts,
        })
    }

    /// Returns the count of the query's bindings and the counts of its restrictions by its
    /// predicate, when it is drawn.
    pub fn counts(&self) -> Option<Split> {
        let Form::Drawn(d) = &self.0 else {
            return None;
        };
        let count = |c: &Comp| {
            let mut r = Render::new(&d.world);
            let gens = r.gens(&c.gens);
            format!("count(1 for {gens}{})", r.filter(&c.filter))
        };
        Some(Split {
            whole: count(&d.comp),
            parts: d.restrictions().map(|c| count(&c)),
        })
    }

    pub fn forms(&self) -> Forms {
        let Form::Drawn(d) = &self.0 else {
            return Forms::default();
        };
        let group: Vec<&E> = d.group.iter().flat_map(|(v, by)| [v, by]).collect();
        let any = |f: &dyn Fn(&E) -> bool| {
            let mut found = false;
            let mut visit = |e: &E| found |= f(e);
            d.comp
                .filter
                .iter()
                .chain([&d.comp.select])
                .chain(group.iter().copied())
                .for_each(|e| e.visit(&mut visit));
            found
        };
        Forms {
            fix: any(&|e| matches!(e, E::Fix(..))),
            group: d.group.is_some(),
            matches: any(&|e| matches!(e, E::Row(..) | E::Shade(..) | E::Opt(..))),
            join: d.comp.gens.len() > 1
                || d.comp.correlated()
                || group.iter().any(|e| e.correlated()),
            float: floats(&d.sel),
        }
    }

    /// Returns the queries one step smaller than this one.
    pub fn shrinks(&self) -> Vec<Query> {
        let Form::Drawn(d) = &self.0 else {
            return Vec::new();
        };
        let with = |f: &dyn Fn(&mut Drawn)| {
            let mut d = d.as_ref().clone();
            f(&mut d);
            Query(Form::Drawn(Box::new(d)))
        };
        let mut out = Vec::new();
        let mut outside: BTreeSet<String> = d.p.reads();
        if let Some((v, by)) = &d.group {
            outside.extend(v.reads());
            outside.extend(by.reads());
            out.push(with(&|d| d.group = None));
            for s in v.shrinks() {
                out.push(with(&|d| d.group = Some((s.clone(), by.clone()))));
            }
            for s in by.shrinks() {
                out.push(with(&|d| d.group = Some((v.clone(), s.clone()))));
            }
        }
        for c in d.comp.shrinks(&outside) {
            out.push(with(&|d| d.comp = c.clone()));
        }
        for p in d.p.shrinks() {
            out.push(with(&|d| d.p = p.clone()));
        }
        out
    }
}

fn floats(t: &T) -> bool {
    match t {
        T::Float | T::FloatSum => true,
        T::Rec(fs) => fs.iter().any(|(_, t)| floats(t)),
        T::Set(t) => floats(t),
        _ => false,
    }
}

impl Drawn {
    fn text(&self) -> String {
        let mut r = Render::new(&self.world);
        match &self.group {
            None => r.comp(&self.comp),
            Some((value, by)) => {
                let gens = r.gens(&self.comp.gens);
                let filter = r.filter(&self.comp.filter);
                format!(
                    "(from {gens}{filter} group {} by {} into g \
                     select {{ key: g.key, n: count(t for t in g), s: sum(t for t in g) }})",
                    r.expr(value),
                    r.expr(by)
                )
            }
        }
    }

    fn restrictions(&self) -> [Comp; 3] {
        [Tried::True, Tried::False, Tried::Faults].map(|t| {
            let tried = E::Tried(Box::new(self.p.clone()), t);
            let filter = match &self.comp.filter {
                Some(f) => E::Bin("and", Box::new(f.clone()), Box::new(tried)),
                None => tried,
            };
            Comp {
                filter: Some(filter),
                ..self.comp.clone()
            }
        })
    }
}

/// Returns the case of `seed`, holding `queries` queries.
pub fn case(seed: u64, queries: usize) -> Case {
    let mut rng = Rng::new(seed);
    let mut world = World::draw(&mut rng);
    let load = world.load(&mut rng);
    let world = Arc::new(world);
    let queries = (0..queries).map(|_| query(&mut rng, &world)).collect();
    Case::drawn(world, load, queries)
}

fn query(rng: &mut Rng, world: &Arc<World>) -> Query {
    let mut g = Gen {
        w: world,
        scope: Vec::new(),
        next: 0,
    };
    let sel = g.select_ty(rng);
    let comp = g.comp(rng, &sel, 3, 3);
    let (p, group) = g.within(&comp.gens, |g| {
        let group = (rng.below(5) == 0).then(|| {
            let by = g.leaf_ty(rng);
            (g.expr(rng, &T::Int, 2), g.expr(rng, &by, 1))
        });
        (g.boolean(rng, 2), group)
    });
    Query(Form::Drawn(Box::new(Drawn {
        world: world.clone(),
        sel,
        comp,
        group,
        p,
    })))
}
