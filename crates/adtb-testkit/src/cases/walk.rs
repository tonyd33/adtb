use std::sync::Arc;

use super::world::{Check, Ctor, Field, Index, Rel, Row, SHADE, State, Sum, T, Val, World};
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
    /// The operators of the migration, in order.
    pub operators: Vec<Operator>,
    /// What a revert of the migration, applied right after it, does.
    pub inverse: Inverse,
    /// The database once the migration applies, as a case of the schema it leaves, with queries
    /// over that schema.
    pub migrated: Case,
    pub writes: Vec<String>,
    /// The database after the writes, with the queries of `migrated`.
    pub written: Case,
}

/// What a revert of a step's migration does when it follows the migration.
#[derive(Clone, Debug)]
pub enum Inverse {
    /// The revert applies and leaves `reverted`: the database before the migration, but for the
    /// values a lossy operator discards, which hold the fills the revert states. Its queries are
    /// those of the case before the migration.
    Restores { revert: String, reverted: Case },
    /// The revert is refused, since an operator of the migration discards data and has no
    /// inverse.
    Refused { revert: String },
}

impl Inverse {
    /// Returns the revert migration.
    pub fn revert(&self) -> &str {
        match self {
            Inverse::Restores { revert, .. } | Inverse::Refused { revert } => revert,
        }
    }
}

/// A schema operator a walk draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    AddRelation,
    AddConstructor,
    Enable,
    Deprecate,
    Disable,
    RemoveConstructor,
    RenameType,
    RenameRelation,
    RenameConstructor,
    /// A split routed by a discriminator into products named as its constructors.
    Split,
    /// A split routed by a `Bool` field.
    SplitByLiteral,
    /// A split into one product, every field renamed.
    SplitRenaming,
    /// A split whose routing drops a field.
    LossySplit,
    Merge,
    Nest,
    Unnest,
    AddField,
    DropField,
    RenameField,
    MoveFieldUp,
    /// A move of a header field down to one constructor.
    MoveFieldDown,
    /// A move of a header field down to several constructors.
    MoveFieldDownMany,
    AlterField,
    AddIndex,
    DropIndex,
    AddUnique,
    DropUnique,
    AddDefault,
    DropDefault,
    AddCheck,
    DropCheck,
    RedefineFn,
}

impl Operator {
    pub const ALL: [Operator; 32] = [
        Operator::AddRelation,
        Operator::AddConstructor,
        Operator::Enable,
        Operator::Deprecate,
        Operator::Disable,
        Operator::RemoveConstructor,
        Operator::RenameType,
        Operator::RenameRelation,
        Operator::RenameConstructor,
        Operator::Split,
        Operator::SplitByLiteral,
        Operator::SplitRenaming,
        Operator::LossySplit,
        Operator::Merge,
        Operator::Nest,
        Operator::Unnest,
        Operator::AddField,
        Operator::DropField,
        Operator::RenameField,
        Operator::MoveFieldUp,
        Operator::MoveFieldDown,
        Operator::MoveFieldDownMany,
        Operator::AlterField,
        Operator::AddIndex,
        Operator::DropIndex,
        Operator::AddUnique,
        Operator::DropUnique,
        Operator::AddDefault,
        Operator::DropDefault,
        Operator::AddCheck,
        Operator::DropCheck,
        Operator::RedefineFn,
    ];
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
    walk_of(seed, steps, queries, &[])
}

/// Returns the walk of `seed` as [`walk`] does, drawing no operator of `skip`.
pub fn walk_of(seed: u64, steps: usize, queries: usize, skip: &[Operator]) -> Walk {
    let mut rng = Rng::new(seed);
    let mut world = World::draw(&mut rng);
    world.load(&mut rng);
    let start = cased(&mut rng, &world, queries);
    let mut before = start.clone();
    let mut out = Vec::new();
    for k in 0..steps {
        let pre = world.clone();
        let mut ops = Vec::new();
        let mut touched = Vec::new();
        for _ in 0..=rng.below(2) {
            let (t, op) = operator(&mut rng, &mut world, &touched, skip);
            touched.push(t);
            ops.push(op);
        }
        let inverse = inverse(pre, &before, &ops, k + 2);
        let migrated = cased(&mut rng, &world, queries);
        let n = 1 + rng.below(4);
        let writes = world.write(&mut rng, n);
        let written = Case {
            load: world.inserts(),
            ..migrated.clone()
        };
        let mut migration = String::from("module app\n");
        for line in ops.iter().flat_map(|op| &op.lines) {
            migration.push_str(line);
            migration.push('\n');
        }
        before = written.clone();
        out.push(Step {
            migration,
            operators: ops.iter().map(|op| op.operator).collect(),
            inverse,
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

/// An operator drawn into a migration: its lines and what its inverse restores.
struct Op {
    operator: Operator,
    lines: Vec<String>,
    loss: Loss,
}

impl Op {
    fn exact(operator: Operator, lines: Vec<String>) -> Op {
        Op {
            operator,
            lines,
            loss: Loss::None,
        }
    }
}

/// The data an operator discards.
enum Loss {
    None,
    /// The values of `field` in the rows of each listed constructor of relation `rel`, which
    /// the inverse restores as the listed value, stated by the revert's `fill`.
    Filled {
        fill: String,
        rel: usize,
        field: String,
        values: Vec<(usize, Val)>,
    },
    /// Data the operator's inverse cannot restore, so a revert of it is refused.
    Unrestored,
}

/// Returns what a revert numbered after migration `number` does, the world `pre` and the case
/// `before` holding the database before that migration's operators `ops`.
fn inverse(mut pre: World, before: &Case, ops: &[Op], number: usize) -> Inverse {
    let mut revert = format!("module app\nrevert {number:04}\n");
    let mut refused = false;
    for op in ops {
        match &op.loss {
            Loss::None => {}
            Loss::Unrestored => refused = true,
            Loss::Filled {
                fill,
                rel,
                field,
                values,
            } => {
                revert.push_str(&format!("  fill {fill}\n"));
                for row in pre.rows.iter_mut().filter(|row| row.rel == *rel) {
                    let Some((_, v)) = values.iter().find(|(c, _)| *c == row.ctor) else {
                        continue;
                    };
                    for (_, held) in row.fields.iter_mut().filter(|(f, _)| f == field) {
                        held.clone_from(v);
                    }
                }
            }
        }
    }
    if refused {
        return Inverse::Refused { revert };
    }
    let load = pre.inserts();
    Inverse::Restores {
        revert,
        reverted: Case::drawn(Arc::new(pre), load, before.queries.clone()),
    }
}

/// The operators a walk draws from, [`Operator::AddRelation`] the fallback when none applies.
const DRAWN: [Operator; 31] = [
    Operator::AddConstructor,
    Operator::Enable,
    Operator::Deprecate,
    Operator::Disable,
    Operator::RemoveConstructor,
    Operator::RenameType,
    Operator::RenameRelation,
    Operator::RenameConstructor,
    Operator::Split,
    Operator::SplitByLiteral,
    Operator::SplitRenaming,
    Operator::LossySplit,
    Operator::Merge,
    Operator::Nest,
    Operator::Unnest,
    Operator::AddField,
    Operator::DropField,
    Operator::RenameField,
    Operator::MoveFieldUp,
    Operator::MoveFieldDown,
    Operator::MoveFieldDownMany,
    Operator::AlterField,
    Operator::AddIndex,
    Operator::DropIndex,
    Operator::AddUnique,
    Operator::DropUnique,
    Operator::AddDefault,
    Operator::DropDefault,
    Operator::AddCheck,
    Operator::DropCheck,
    Operator::RedefineFn,
];

/// Applies a schema operator not in `skip` drawn at random to a relation of `w`, and returns
/// the relation it touches, `None` for a function, and the operator. A relation or function in
/// `touched` is not touched again, so an operator of a migration names nothing an earlier one
/// of it touches, but for the types a merge or a nest adds. A relation takes one of the
/// operators [`undoing`] names for it one time in three.
fn operator(
    rng: &mut Rng,
    w: &mut World,
    touched: &[Option<usize>],
    skip: &[Operator],
) -> (Option<usize>, Op) {
    for _ in 0..24 {
        let r = rng.below(w.rels.len() as u64) as usize;
        let undone = undoing(w, r);
        let operator = match rng.pick_some(&undone) {
            Some(o) if rng.below(3) == 0 => *o,
            _ => *rng.pick(&DRAWN),
        };
        if rng.below(20) == 0 {
            break;
        }
        let at = (operator != Operator::RedefineFn).then_some(r);
        if skip.contains(&operator) || touched.contains(&at) {
            continue;
        }
        if let Some(op) = draw(operator, rng, w, r) {
            return (at, op);
        }
    }
    let op = add_relation(rng, w);
    (Some(w.rels.len() - 1), op)
}

/// Returns the operators that take back what an earlier operator left on relation `r`: an
/// unnest of a constructor holding one value of a declared sum, a split of one holding a value
/// of a declared sum of fieldless constructors, a remove of an empty deprecated constructor, and
/// a drop of a unique, default or check. Returns a merge too when a check ranges over one
/// constructor alone, which the merge carries.
fn undoing(w: &World, r: usize) -> Vec<Operator> {
    let rel = &w.rels[r];
    let mut out = Vec::new();
    for (k, c) in rel.ctors.iter().enumerate() {
        if let [
            Field {
                ty: T::Named(_), ..
            },
        ] = c.fields.as_slice()
        {
            out.push(Operator::Unnest);
        }
        let tagged = |f: &Field| matches!(f.ty, T::Named(s) if w.sums[s].ctors.iter().all(|d| d.fields.is_empty()));
        if c.fields.iter().any(tagged) {
            out.push(Operator::Split);
        }
        if c.state == State::Deprecated && !w.holds_rows(r, k) {
            out.push(Operator::RemoveConstructor);
        }
    }
    for (some, o) in [
        (rel.checks.iter().any(|k| k.ctor.is_some()), Operator::Merge),
        (!rel.uniques.is_empty(), Operator::DropUnique),
        (!rel.defaults.is_empty(), Operator::DropDefault),
        (!rel.checks.is_empty(), Operator::DropCheck),
    ] {
        if some {
            out.push(o);
        }
    }
    out
}

fn draw(operator: Operator, rng: &mut Rng, w: &mut World, r: usize) -> Option<Op> {
    let lines = match operator {
        Operator::AddRelation => return Some(add_relation(rng, w)),
        Operator::AddConstructor => add_constructor(rng, w, r),
        Operator::Enable | Operator::Deprecate | Operator::Disable => state(rng, w, r, operator),
        Operator::RemoveConstructor => remove_constructor(rng, w, r),
        Operator::RenameType => rename_type(w, r),
        Operator::RenameRelation => rename_relation(w, r),
        Operator::RenameConstructor => rename_constructor(rng, w, r),
        Operator::Split => split(rng, w, r, Route::Discriminator),
        Operator::SplitByLiteral => split(rng, w, r, Route::Literal),
        Operator::SplitRenaming => split(rng, w, r, Route::Renaming),
        Operator::LossySplit => {
            return lossy_split(rng, w, r).map(|lines| Op {
                operator,
                lines,
                loss: Loss::Unrestored,
            });
        }
        Operator::Merge => merge(rng, w, r),
        Operator::Nest => nest(rng, w, r),
        Operator::Unnest => unnest(rng, w, r),
        Operator::AddField => add_field(rng, w, r),
        Operator::DropField => return drop_field(rng, w, r),
        Operator::RenameField => rename_field(rng, w, r),
        Operator::MoveFieldUp => move_up(rng, w, r),
        Operator::MoveFieldDown => return move_down(rng, w, r, false),
        Operator::MoveFieldDownMany => return move_down(rng, w, r, true),
        Operator::AlterField => alter_field(rng, w, r),
        Operator::AddIndex => add_index(rng, w, r),
        Operator::DropIndex => drop_index(rng, w, r),
        Operator::AddUnique => add_unique(rng, w, r),
        Operator::DropUnique => drop_unique(rng, w, r),
        Operator::AddDefault => add_default(rng, w, r),
        Operator::DropDefault => drop_default(rng, w, r),
        Operator::AddCheck => add_check(rng, w, r),
        Operator::DropCheck => drop_check(rng, w, r),
        Operator::RedefineFn => Some(redefine_fn(w)),
    }?;
    Some(Op::exact(operator, lines))
}

/// The field types an operator adds.
const ADDED: [T; 6] = [T::Int, T::Text, T::Bool, T::Shade, T::OptInt, T::Float];

fn add_relation(rng: &mut Rng, w: &mut World) -> Op {
    let rel = Rel::draw(rng, w.rels.len());
    let lines = vec![
        format!("add {}", rel.type_decl(w)),
        format!("add {}", rel.relation_decl()),
    ];
    w.rels.push(rel);
    Op::exact(Operator::AddRelation, lines)
}

/// Returns the constructor fields a drawn constructor or field takes, named freshly.
fn fields(rng: &mut Rng, w: &mut World, n: u64) -> Vec<Field> {
    (0..n)
        .map(|_| Field::new(w.fresh("f"), rng.pick(&ADDED).clone()))
        .collect()
}

/// Returns `fs` named freshly.
fn renamed(w: &mut World, fs: &[Field]) -> Vec<Field> {
    fs.iter()
        .map(|f| Field::new(w.fresh("f"), f.ty.clone()))
        .collect()
}

fn add_constructor(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    if !w.rels[r].sum {
        return None;
    }
    let name = w.fresh(&format!("C{r}"));
    let n = 1 + rng.below(2);
    let fs = fields(rng, w, n);
    let mut ctor = Ctor {
        name,
        fields: fs,
        state: State::Declared,
    };
    let rel = &w.rels[r];
    let mut lines = vec![format!("add constructor {}.{}", rel.ty, ctor.decl(w))];
    if rng.below(2) == 0 {
        lines.push(format!("enable {}.{}", rel.name, ctor.name));
        ctor.state = State::Enabled;
    }
    w.rels[r].ctors.push(ctor);
    Some(lines)
}

/// Moves a constructor of a sum `r` to the write state `operator` names, from a state that
/// operator leaves.
fn state(rng: &mut Rng, w: &mut World, r: usize, operator: Operator) -> Option<Vec<String>> {
    if !w.rels[r].sum {
        return None;
    }
    let (verb, to) = match operator {
        Operator::Enable => ("enable", State::Enabled),
        Operator::Deprecate => ("deprecate", State::Deprecated),
        _ => ("disable", State::Declared),
    };
    let from: Vec<usize> = (0..w.rels[r].ctors.len())
        .filter(|c| {
            let s = w.rels[r].ctors[*c].state;
            match to {
                State::Enabled => s != State::Enabled,
                State::Deprecated => s == State::Enabled,
                State::Declared => s == State::Enabled && !w.holds_rows(r, *c),
            }
        })
        .collect();
    let c = *rng.pick_some(&from)?;
    let rel = &mut w.rels[r];
    rel.ctors[c].state = to;
    Some(vec![format!("{verb} {}.{}", rel.name, rel.ctors[c].name)])
}

fn remove_constructor(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let rel = &w.rels[r];
    if !rel.sum || rel.ctors.len() < 2 {
        return None;
    }
    let gone: Vec<usize> = (0..rel.ctors.len())
        .filter(|c| rel.ctors[*c].state == State::Deprecated)
        .filter(|c| !w.holds_rows(r, *c) && !rel.scopes(*c))
        .collect();
    let c = *rng.pick_some(&gone)?;
    let line = format!("remove constructor {}.{}", rel.ty, rel.ctors[c].name);
    w.replace(r, &[c], Vec::new(), |_| {
        unreachable!("a removed constructor holds no row")
    });
    Some(vec![line])
}

fn rename_type(w: &mut World, r: usize) -> Option<Vec<String>> {
    if !w.rels[r].sum {
        return None;
    }
    let to = w.fresh("T");
    let from = std::mem::replace(&mut w.rels[r].ty, to.clone());
    Some(vec![format!("rename type {from} to {to}")])
}

fn rename_relation(w: &mut World, r: usize) -> Option<Vec<String>> {
    let to = w.fresh("r");
    let from = std::mem::replace(&mut w.rels[r].name, to.clone());
    let (old, new) = (format!("{from}.key("), format!("{to}.key("));
    for (_, v) in w.rows.iter_mut().flat_map(|row| &mut row.fields) {
        if let Val::Lit(s) = v
            && let Some(rest) = s.strip_prefix(&old)
        {
            *s = format!("{new}{rest}");
        }
    }
    Some(vec![format!("rename relation {from} to {to}")])
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

/// Returns the fields of constructor `c` of `rel`, or of the header.
fn fields_of(rel: &Rel, c: Option<usize>) -> &[Field] {
    match c {
        Some(c) => &rel.ctors[c].fields,
        None => &rel.header,
    }
}

fn fields_mut(rel: &mut Rel, c: Option<usize>) -> &mut Vec<Field> {
    match c {
        Some(c) => &mut rel.ctors[c].fields,
        None => &mut rel.header,
    }
}

/// Returns whether row `row` holds the fields of constructor `c`, or of the header.
fn within(row: &Row, r: usize, c: Option<usize>) -> bool {
    row.rel == r && c.is_none_or(|c| row.ctor == c)
}

fn add_field(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let c = place(rng, &w.rels[r]);
    let f = fields(rng, w, 1).remove(0);
    let fill = w.val(rng, &f.ty, &[]);
    let op = format!(
        "add field {}: {} fill {fill}",
        path(&w.rels[r], c, &f.name),
        f.ty.name(w)
    );
    for row in w.rows.iter_mut().filter(|row| within(row, r, c)) {
        row.fields.push((f.name.clone(), fill.clone()));
    }
    fields_mut(&mut w.rels[r], c).push(f);
    Some(vec![op])
}

/// Returns a field of `rel` that holds no key and that no index, unique, default or check
/// names, drawn at random, with its constructor, or `None` for the header.
fn free(rng: &mut Rng, rel: &Rel) -> Option<(Option<usize>, Field)> {
    let c = place(rng, rel);
    let free: Vec<&Field> = fields_of(rel, c)
        .iter()
        .filter(|f| !matches!(f.ty, T::Key(_)))
        .filter(|f| !rel.attached(c, &f.name))
        .collect();
    rng.pick_some(&free).map(|f| (c, (*f).clone()))
}

fn drop_field(rng: &mut Rng, w: &mut World, r: usize) -> Option<Op> {
    let (c, f) = free(rng, &w.rels[r])?;
    let at = path(&w.rels[r], c, &f.name);
    let fill = w.val(rng, &f.ty, &[]);
    let ctors: Vec<usize> = match c {
        Some(c) => vec![c],
        None => (0..w.rels[r].ctors.len()).collect(),
    };
    for row in w.rows.iter_mut().filter(|row| within(row, r, c)) {
        row.fields.retain(|(g, _)| *g != f.name);
    }
    fields_mut(&mut w.rels[r], c).retain(|g| g.name != f.name);
    Some(Op {
        operator: Operator::DropField,
        lines: vec![format!("drop field {at}")],
        loss: Loss::Filled {
            fill: format!("{at} = {fill}"),
            rel: r,
            field: f.name,
            values: ctors.into_iter().map(|c| (c, fill.clone())).collect(),
        },
    })
}

fn rename_field(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let (c, f) = free(rng, &w.rels[r])?;
    let g = w.fresh("f");
    let op = format!("rename field {} to {g}", path(&w.rels[r], c, &f.name));
    for row in w.rows.iter_mut().filter(|row| within(row, r, c)) {
        for (name, _) in row.fields.iter_mut().filter(|(name, _)| *name == f.name) {
            name.clone_from(&g);
        }
    }
    for field in fields_mut(&mut w.rels[r], c)
        .iter_mut()
        .filter(|field| field.name == f.name)
    {
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

/// Returns a constructor of a sum `r` drawn at random that no index, unique or default ranges
/// over alone.
fn unscoped(rng: &mut Rng, rel: &Rel) -> Option<usize> {
    if !rel.sum {
        return None;
    }
    let cs: Vec<usize> = (0..rel.ctors.len()).filter(|c| !rel.scopes(*c)).collect();
    rng.pick_some(&cs).copied()
}

/// Returns the routing pattern binding each of `fs` by its name, under constructor `c` and with
/// field `fixed` fixed to its literal.
fn pattern(c: &str, fixed: Option<(&str, &str)>, fs: &[Field]) -> String {
    let mut parts: Vec<String> = fixed.iter().map(|(f, v)| format!("{f}: {v}")).collect();
    parts.extend(fs.iter().map(|f| f.name.clone()));
    if parts.is_empty() {
        format!("{c} {{}}")
    } else {
        format!("{c} {{ {} }}", parts.join(", "))
    }
}

/// Returns the value of constructor `c` of a routing's result, each field of `to` taking the
/// expression of `from` paired with it.
fn result(c: &str, to: &[Field], from: &[String]) -> String {
    let parts: Vec<String> = to
        .iter()
        .zip(from)
        .map(|(t, e)| format!("{}: {e}", t.name))
        .collect();
    if parts.is_empty() {
        format!("{c} {{}}")
    } else {
        format!("{c} {{ {} }}", parts.join(", "))
    }
}

/// The fields of a row, each with its value.
type Fields = Vec<(String, Val)>;

/// Returns `row`'s header fields of relation `rel`, and its other fields.
fn parts(rel: &Rel, row: &Row) -> (Fields, Fields) {
    row.fields
        .iter()
        .cloned()
        .partition(|(f, _)| rel.header.iter().any(|h| h.name == *f))
}

/// How an invertible split routes the rows of its constructor.
#[derive(Clone, Copy)]
enum Route {
    /// By a field of a declared sum of fieldless constructors, into products named as those.
    Discriminator,
    /// By a `Bool` field, into two products.
    Literal,
    /// Into one product, every field renamed.
    Renaming,
}

/// Splits a constructor of a sum `r` by an invertible routing, each field but the one it routes
/// by carried whole under a fresh name.
fn split(rng: &mut Rng, w: &mut World, r: usize, route: Route) -> Option<Vec<String>> {
    let c = unscoped(rng, &w.rels[r])?;
    let rel = w.rels[r].clone();
    let source = rel.ctors[c].clone();
    let (by, routes): (Option<usize>, Vec<(Option<Val>, String)>) = match route {
        Route::Discriminator => {
            let (b, s) = source
                .fields
                .iter()
                .enumerate()
                .find_map(|(k, f)| match f.ty {
                    T::Named(s) if w.sums[s].ctors.iter().all(|d| d.fields.is_empty()) => {
                        Some((k, s))
                    }
                    _ => None,
                })?;
            let names: Vec<String> = w.sums[s].ctors.iter().map(|d| d.name.clone()).collect();
            if rel.ctors.iter().any(|d| names.contains(&d.name)) {
                return None;
            }
            let routes = names
                .into_iter()
                .map(|n| (Some(Val::Ctor(n.clone(), Vec::new())), n))
                .collect();
            (Some(b), routes)
        }
        Route::Literal => {
            let b = source.fields.iter().position(|f| f.ty == T::Bool)?;
            let routes = ["true", "false"]
                .into_iter()
                .map(|v| (Some(Val::Lit(v.into())), w.fresh(&format!("C{r}"))))
                .collect();
            (Some(b), routes)
        }
        Route::Renaming => (None, vec![(None, w.fresh(&format!("C{r}")))]),
    };
    let rest: Vec<Field> = source
        .fields
        .iter()
        .enumerate()
        .filter(|(k, _)| Some(*k) != by)
        .map(|(_, f)| f.clone())
        .collect();
    let names: Vec<String> = rest.iter().map(|f| f.name.clone()).collect();
    let mut products = Vec::new();
    let mut arms = Vec::new();
    for (fixed, name) in &routes {
        let p = Ctor {
            name: name.clone(),
            fields: renamed(w, &rest),
            state: source.state,
        };
        let fixed = fixed.as_ref().map(ToString::to_string);
        let fixed = by
            .zip(fixed.as_deref())
            .map(|(b, v)| (source.fields[b].name.as_str(), v));
        arms.push(format!(
            "    | {} -> {}",
            pattern(&source.name, fixed, &rest),
            result(&p.name, &p.fields, &names)
        ));
        products.push(p);
    }
    let into: Vec<String> = products.iter().map(|p| p.decl(w)).collect();
    let mut lines = vec![format!(
        "split constructor {}.{} into {}\n  by match",
        rel.ty,
        source.name,
        into.join(" | ")
    )];
    lines.extend(arms);
    let lines = vec![lines.join("\n")];
    let targets = products.clone();
    w.replace(r, &[c], products, |row| {
        let (mut header, own) = parts(&rel, row);
        let held = by.and_then(|b| own.iter().find(|(f, _)| *f == source.fields[b].name));
        let p = routes
            .iter()
            .position(|(v, _)| v.is_none() || v.as_ref() == held.map(|(_, h)| h))
            .expect("a routing covers every row of its constructor");
        for (f, g) in rest.iter().zip(&targets[p].fields) {
            if let Some((_, v)) = own.iter().find(|(n, _)| *n == f.name) {
                header.push((g.name.clone(), v.clone()));
            }
        }
        (p, header)
    });
    Some(lines)
}

/// Splits a constructor of a sum `r` with at least one field into one product without its last
/// field.
fn lossy_split(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let c = unscoped(rng, &w.rels[r])?;
    let source = w.rels[r].ctors[c].clone();
    let (_, kept) = source.fields.split_last()?;
    let kept = kept.to_vec();
    let p = Ctor {
        name: w.fresh(&format!("C{r}")),
        fields: renamed(w, &kept),
        state: source.state,
    };
    let from: Vec<String> = kept.iter().map(|f| format!("x.{}", f.name)).collect();
    let rel = w.rels[r].clone();
    let line = format!(
        "split constructor {}.{} into {}\n  by match\n    | {} x -> {}",
        rel.ty,
        source.name,
        p.decl(w),
        source.name,
        result(&p.name, &p.fields, &from)
    );
    let fields = p.fields.clone();
    w.replace(r, &[c], vec![p], |row| {
        let (mut header, own) = parts(&rel, row);
        for (f, g) in kept.iter().zip(&fields) {
            if let Some((_, v)) = own.iter().find(|(n, _)| *n == f.name) {
                header.push((g.name.clone(), v.clone()));
            }
        }
        (0, header)
    });
    Some(vec![line])
}

/// Returns two constructors of a sum `r` drawn at random, in declaration order, that may be
/// merged or nested: neither deprecated, no index, unique or default over one alone, no check
/// over one alone unless `checks` carry, and neither named as a constructor of a sum the walk
/// declares. When `checks` carry, one is a constructor a check ranges over alone half the time.
fn pair(rng: &mut Rng, w: &World, r: usize, checks: bool) -> Option<(usize, usize)> {
    let rel = &w.rels[r];
    if !rel.sum || rel.ctors.len() < 2 {
        return None;
    }
    let ok = |c: usize| {
        let ctor = &rel.ctors[c];
        let scoped = if checks {
            rel.indexes_alone(c)
        } else {
            rel.scopes(c)
        };
        ctor.state != State::Deprecated
            && !scoped
            && !w
                .sums
                .iter()
                .any(|s| s.ctors.iter().any(|d| d.name == ctor.name))
    };
    let checked: Vec<usize> = rel.checks.iter().filter_map(|k| k.ctor).collect();
    let a = match rng.pick_some(&checked) {
        Some(&c) if checks && rng.below(2) == 0 => c,
        _ => rng.below(rel.ctors.len() as u64) as usize,
    };
    let b = rng.below(rel.ctors.len() as u64) as usize;
    (a != b && ok(a) && ok(b)).then(|| (a.min(b), a.max(b)))
}

/// Returns the state of the product of a merge or nest of constructors in states `a` and `b`.
fn joined(a: State, b: State) -> State {
    if a == State::Enabled && b == State::Enabled {
        State::Enabled
    } else {
        State::Declared
    }
}

/// Merges two constructors of a sum `r` into one, its discriminator of a sum added alongside,
/// each source's fields carried whole and the other source's fields filled with literals.
fn merge(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let (a, b) = pair(rng, w, r, true)?;
    let sources = [w.rels[r].ctors[a].clone(), w.rels[r].ctors[b].clone()];
    let tag_ty = Sum {
        name: w.fresh("D"),
        ctors: sources
            .iter()
            .map(|s| Ctor {
                name: s.name.clone(),
                fields: Vec::new(),
                state: State::Enabled,
            })
            .collect(),
    };
    let tag = Field::new(w.fresh("f"), T::Named(w.sums.len()));
    let carried = [
        renamed(w, &sources[0].fields),
        renamed(w, &sources[1].fields),
    ];
    let fills: Vec<Vec<Val>> = carried
        .iter()
        .map(|fs| fs.iter().map(|f| w.val(rng, &f.ty, &[])).collect())
        .collect();
    let mut fields = vec![tag.clone()];
    fields.extend(carried.iter().flatten().cloned());
    let merged = Ctor {
        name: w.fresh(&format!("C{r}")),
        fields,
        state: joined(sources[0].state, sources[1].state),
    };
    let sum_decl = tag_ty.decl(w);
    w.sums.push(tag_ty);
    let mut arms = Vec::new();
    for (k, s) in sources.iter().enumerate() {
        let mut from = vec![s.name.clone()];
        for (j, fill) in fills.iter().enumerate() {
            if j == k {
                from.extend(s.fields.iter().map(|f| format!("x.{}", f.name)));
            } else {
                from.extend(fill.iter().map(ToString::to_string));
            }
        }
        arms.push(format!(
            "    | {} x -> {}",
            s.name,
            result(&merged.name, &merged.fields, &from)
        ));
    }
    let rel = w.rels[r].clone();
    let lines = vec![
        format!("add {sum_decl}"),
        format!(
            "merge constructor {}.({} | {}) into {}\n  by match\n{}",
            rel.ty,
            sources[0].name,
            sources[1].name,
            merged.decl(w),
            arms.join("\n")
        ),
    ];
    w.replace(r, &[a, b], vec![merged], |row| {
        let k = usize::from(row.ctor == b);
        let (mut out, own) = parts(&rel, row);
        out.push((
            tag.name.clone(),
            Val::Ctor(sources[k].name.clone(), Vec::new()),
        ));
        for (j, fs) in carried.iter().enumerate() {
            for (i, g) in fs.iter().enumerate() {
                let v = if j == k {
                    let name = &sources[k].fields[i].name;
                    own.iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, v)| v.clone())
                        .expect("a row holds every field of its constructor")
                } else {
                    fills[j][i].clone()
                };
                out.push((g.name.clone(), v));
            }
        }
        (0, out)
    });
    Some(lines)
}

/// Nests two constructors of a sum `r` into one holding a value of a sum added alongside, of
/// constructors of the same names and fields.
fn nest(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let (a, b) = pair(rng, w, r, false)?;
    let sources = [w.rels[r].ctors[a].clone(), w.rels[r].ctors[b].clone()];
    let held = Sum {
        name: w.fresh("N"),
        ctors: sources
            .iter()
            .map(|s| Ctor {
                state: State::Enabled,
                ..s.clone()
            })
            .collect(),
    };
    let field = Field::new(w.fresh("f"), T::Named(w.sums.len()));
    let nested = Ctor {
        name: w.fresh(&format!("C{r}")),
        fields: vec![field.clone()],
        state: joined(sources[0].state, sources[1].state),
    };
    let sum_decl = held.decl(w);
    w.sums.push(held);
    let rel = w.rels[r].clone();
    let lines = vec![
        format!("add {sum_decl}"),
        format!(
            "nest constructor {}.({} | {}) into {}",
            rel.ty,
            sources[0].name,
            sources[1].name,
            nested.decl(w)
        ),
    ];
    w.replace(r, &[a, b], vec![nested], |row| {
        let k = usize::from(row.ctor == b);
        let (mut out, own) = parts(&rel, row);
        out.push((field.name.clone(), Val::Ctor(sources[k].name.clone(), own)));
        (0, out)
    });
    Some(lines)
}

/// Unnests a constructor of a sum `r` holding one field of a sum the walk declares, when no
/// other constructor of `r` is named as a constructor of that sum.
fn unnest(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let c = unscoped(rng, &w.rels[r])?;
    let rel = w.rels[r].clone();
    let [
        Field {
            ty: T::Named(s), ..
        },
    ] = rel.ctors[c].fields.as_slice()
    else {
        return None;
    };
    let held = w.sums[*s].clone();
    if rel
        .ctors
        .iter()
        .any(|d| held.ctors.iter().any(|h| h.name == d.name))
    {
        return None;
    }
    let state = rel.ctors[c].state;
    let products: Vec<Ctor> = held
        .ctors
        .iter()
        .map(|h| Ctor { state, ..h.clone() })
        .collect();
    let line = format!("unnest constructor {}.{}", rel.ty, rel.ctors[c].name);
    w.replace(r, &[c], products, |row| {
        let (mut out, own) = parts(&rel, row);
        let [(_, Val::Ctor(name, fs))] = own.as_slice() else {
            unreachable!("a nested row holds one value of the nested sum")
        };
        let p = held
            .ctors
            .iter()
            .position(|h| h.name == *name)
            .expect("a nested value is of a constructor of its sum");
        out.extend(fs.iter().cloned());
        (p, out)
    });
    Some(vec![line])
}

/// Returns a `fill match` over the constructors `cs` of `rel`, with the literal each takes.
fn fill_match(rng: &mut Rng, w: &World, rel: &Rel, cs: &[usize], ty: &T) -> (String, Vec<Val>) {
    let mut text = String::from("match");
    let mut vals = Vec::new();
    for c in cs {
        let v = w.val(rng, ty, &[]);
        text.push_str(&format!("\n    | {} x -> {v}", rel.ctors[*c].name));
        vals.push(v);
    }
    (text, vals)
}

/// Moves a field of a constructor of a sum `r` that no other constructor has to the header.
fn move_up(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let rel = &w.rels[r];
    if !rel.sum || rel.ctors.len() < 2 {
        return None;
    }
    let (Some(c), f) = free(rng, rel)? else {
        return None;
    };
    let others: Vec<usize> = (0..rel.ctors.len()).filter(|d| *d != c).collect();
    if others
        .iter()
        .any(|d| rel.ctors[*d].fields.iter().any(|g| g.name == f.name))
    {
        return None;
    }
    let (fill, vals) = fill_match(rng, w, rel, &others, &f.ty);
    let line = format!(
        "move field {} to {} fill {fill}",
        path(rel, Some(c), &f.name),
        rel.ty
    );
    for row in w.rows.iter_mut().filter(|row| row.rel == r) {
        if let Some(k) = others.iter().position(|d| *d == row.ctor) {
            row.fields.push((f.name.clone(), vals[k].clone()));
        }
    }
    let rel = &mut w.rels[r];
    rel.ctors[c].fields.retain(|g| g.name != f.name);
    rel.header.push(f);
    Some(vec![line])
}

/// Moves a header field of a sum `r` down to one of its constructors, or to several but not all
/// when `many` holds, losing its values in the others.
fn move_down(rng: &mut Rng, w: &mut World, r: usize, many: bool) -> Option<Op> {
    let rel = &w.rels[r];
    if !rel.sum || rel.ctors.len() < 2 + usize::from(many) {
        return None;
    }
    let free: Vec<&Field> = rel
        .header
        .iter()
        .filter(|f| !matches!(f.ty, T::Key(_)) && !rel.attached(None, &f.name))
        .collect();
    let f = (*rng.pick_some(&free)?).clone();
    let n = rel.ctors.len();
    let k = if many {
        2 + rng.below(n as u64 - 2) as usize
    } else {
        1
    };
    let mut to: Vec<usize> = (0..n).collect();
    while to.len() > k {
        to.remove(rng.below(to.len() as u64) as usize);
    }
    let lost: Vec<usize> = (0..n).filter(|c| !to.contains(c)).collect();
    let names: Vec<&str> = to.iter().map(|c| rel.ctors[*c].name.as_str()).collect();
    let line = format!(
        "move field {} to {}.({})",
        path(rel, None, &f.name),
        rel.ty,
        names.join(" | ")
    );
    let (fill, vals) = fill_match(rng, w, rel, &lost, &f.ty);
    let fill = format!("{} = {fill}", path(rel, Some(to[0]), &f.name));
    for row in w.rows.iter_mut().filter(|row| row.rel == r) {
        if lost.contains(&row.ctor) {
            row.fields.retain(|(g, _)| *g != f.name);
        }
    }
    let rel = &mut w.rels[r];
    rel.header.retain(|g| g.name != f.name);
    for c in &to {
        rel.ctors[*c].fields.push(f.clone());
    }
    Some(Op {
        operator: if many {
            Operator::MoveFieldDownMany
        } else {
            Operator::MoveFieldDown
        },
        lines: vec![line],
        loss: Loss::Filled {
            fill,
            rel: r,
            field: f.name,
            values: lost.into_iter().zip(vals).collect(),
        },
    })
}

fn alter_field(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let rel = &mut w.rels[r];
    let keys: Vec<usize> = (0..rel.header.len())
        .filter(|k| matches!(rel.header[*k].ty, T::Key(_)))
        .collect();
    let k = *rng.pick_some(&keys)?;
    let action = match rel.header[k].on_delete {
        Some("cascade") => "restrict",
        _ => "cascade",
    };
    rel.header[k].on_delete = Some(action);
    Some(vec![format!(
        "alter field {}.{} on delete {action}",
        rel.ty, rel.header[k].name
    )])
}

fn add_index(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let rel = &mut w.rels[r];
    let ctor = place(rng, rel);
    let names: Vec<&Field> = fields_of(rel, ctor)
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

/// Adds a unique over a free integer or text field whose values the rows in its scope hold
/// once each.
fn add_unique(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let (ctor, f) = free(rng, &w.rels[r])?;
    if !matches!(f.ty, T::Int | T::Text) {
        return None;
    }
    let mut seen = w.seen_in(r, ctor, &f.name);
    let n = seen.len();
    seen.sort();
    seen.dedup();
    if seen.len() != n {
        return None;
    }
    let u = Index {
        field: f.name,
        ctor,
    };
    let rel = &mut w.rels[r];
    let op = format!("add unique ({}) in {}", u.field, rel.index_target(&u));
    rel.uniques.push(u);
    Some(vec![op])
}

fn drop_unique(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let rel = &mut w.rels[r];
    if rel.uniques.is_empty() {
        return None;
    }
    let k = rng.below(rel.uniques.len() as u64) as usize;
    let u = rel.uniques.remove(k);
    Some(vec![format!(
        "drop unique ({}) in {}",
        u.field,
        rel.index_target(&u)
    )])
}

fn add_default(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let (ctor, f) = free(rng, &w.rels[r])?;
    if !f.ty.scalar() {
        return None;
    }
    let v = w.val(rng, &f.ty, &[]).to_string();
    let d = Index {
        field: f.name,
        ctor,
    };
    let rel = &mut w.rels[r];
    let op = format!("add default {}.{} = {v}", rel.name, rel.default_target(&d));
    rel.defaults.push((d, v));
    Some(vec![op])
}

fn drop_default(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let rel = &mut w.rels[r];
    if rel.defaults.is_empty() {
        return None;
    }
    let k = rng.below(rel.defaults.len() as u64) as usize;
    let (d, _) = rel.defaults.remove(k);
    Some(vec![format!(
        "drop default {}.{}",
        rel.name,
        rel.default_target(&d)
    )])
}

/// Adds a check bounding a free integer header field below by the least integer a case draws,
/// over the relation or, of a sum, over one constructor.
fn add_check(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let rel = &w.rels[r];
    let ints: Vec<&Field> = rel
        .header
        .iter()
        .filter(|f| f.ty == T::Int && !rel.attached(None, &f.name))
        .collect();
    let field = rng.pick_some(&ints)?.name.clone();
    let ctor = (rel.sum && rng.below(2) == 0).then(|| rng.below(rel.ctors.len() as u64) as usize);
    let k = Check {
        name: w.fresh("k"),
        field,
        ctor,
    };
    let rel = &mut w.rels[r];
    let op = format!("add {}", rel.check_decl(&k));
    rel.checks.push(k);
    Some(vec![op])
}

fn drop_check(rng: &mut Rng, w: &mut World, r: usize) -> Option<Vec<String>> {
    let rel = &mut w.rels[r];
    if rel.checks.is_empty() {
        return None;
    }
    let k = rng.below(rel.checks.len() as u64) as usize;
    let gone = rel.checks.remove(k);
    Some(vec![format!("drop check {}", gone.name)])
}

fn redefine_fn(w: &mut World) -> Vec<String> {
    w.shade = (w.shade + 1) % SHADE.len();
    vec![format!("redefine {}", w.shade_decl())]
}
