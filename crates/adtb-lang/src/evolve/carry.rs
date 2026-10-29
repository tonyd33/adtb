use super::grant::IdKind;
use super::*;

/// How a structural operator carries the grants on the constructors it replaces to the
/// constructors it makes.
#[derive(Clone, Debug)]
pub(super) struct Carry {
    what: Structural,
    span: Span,
    /// The type's path, `m::T`.
    ty: String,
    /// The relation variables over the type, by id, with their names.
    relations: Vec<(u64, String)>,
    /// The sources each named set of a relation over the type holds, by set id.
    sets: BTreeMap<u64, BTreeSet<u64>>,
    /// The replaced constructors, by id, with their names.
    sources: Vec<(u64, String)>,
    products: Vec<String>,
    /// Each source with a product its rows go to.
    reaches: BTreeSet<(u64, String)>,
    /// For each source, product and field of the product, how each routing of the source's
    /// rows gives the field: the id of the field it carries whole, `None` when it computes it.
    given: BTreeMap<(u64, String, String), Vec<Option<u64>>>,
    /// The fields of the products that tell which source a row came from.
    discriminators: Vec<String>,
    /// The fields of the sources whose values the routing reads to choose a row's product.
    matched: BTreeSet<u64>,
}

/// A structural operator that carries grants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Structural {
    Split,
    Merge,
    Nest,
    Unnest,
}

impl Structural {
    fn name(self) -> &'static str {
        match self {
            Structural::Split => "split",
            Structural::Merge => "merge",
            Structural::Nest => "nest",
            Structural::Unnest => "unnest",
        }
    }
}

/// The kind of a privilege.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Read,
    Insert,
    Update,
    Delete,
    Retag,
    Call,
}

impl Kind {
    fn of(p: &Granted) -> Kind {
        match p {
            Granted::Read(..) => Kind::Read,
            Granted::Insert(_) => Kind::Insert,
            Granted::Update(..) => Kind::Update,
            Granted::Delete(_) => Kind::Delete,
            Granted::Retag(..) => Kind::Retag,
            Granted::CallFn(_) => Kind::Call,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Read => "read",
            Kind::Insert => "insert",
            Kind::Update => "update",
            Kind::Delete => "delete",
            Kind::Retag => "retag",
            Kind::Call => "call",
        }
    }
}

/// For each source, product and field of the product, by name, how each routing of the
/// source's rows gives the field: the source's field it carries whole, `None` when it computes
/// it.
pub(super) type Given = BTreeMap<(String, String, String), Vec<Option<String>>>;

/// Each source with a product its rows go to, by name.
pub(super) type Reaches = BTreeSet<(String, String)>;

/// A grant a structural operator of the migration being applied could not carry to a
/// constructor it made, which a grant operator later in the migration must state.
#[derive(Clone, Debug)]
pub(super) struct Unstated {
    role: String,
    relation: u64,
    kind: Kind,
    product: u64,
    pub(super) span: Span,
    pub(super) message: String,
}

/// What one routing arm does: the source it matches (any, when `None`), and the product it
/// makes with each field it gives and the source field it carries whole into it; `to` is `None`
/// when the arm's result is not a product written out.
pub(super) struct ArmImage {
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
    pub(super) given: Vec<(String, Option<String>)>,
}

fn targets(p: &Granted) -> Vec<&GrantTarget> {
    match p {
        Granted::Read(t, _) | Granted::Update(t, _) | Granted::Insert(t) | Granted::Delete(t) => {
            vec![t]
        }
        Granted::Retag(a, b, _) => vec![a, b],
        Granted::CallFn(_) => Vec::new(),
    }
}

impl Unstated {
    fn same(&self, o: &Unstated) -> bool {
        self.role == o.role
            && self.relation == o.relation
            && self.kind == o.kind
            && self.product == o.product
    }

    /// Returns whether privilege `p` granted to `role`, covering constructors `covered`, states
    /// the grant this one lacks.
    pub(super) fn stated_by(&self, role: &str, p: &Granted, covered: &BTreeSet<u64>) -> bool {
        role == self.role
            && Kind::of(p) == self.kind
            && covered.contains(&self.product)
            && targets(p).iter().any(|t| t.relation == self.relation)
    }
}

/// Returns where each routing arm of `images` gives each field of the products `products`, the
/// names of their fields with each, for rows of `sources`.
pub(super) fn routed(
    images: &[ArmImage],
    sources: &[String],
    products: &[(String, Vec<String>)],
) -> (Reaches, Given) {
    let mut reaches = Reaches::new();
    let mut given = Given::new();
    for img in images {
        let froms: Vec<&String> = match &img.from {
            Some(f) => vec![f],
            None => sources.iter().collect(),
        };
        for s in froms {
            for (p, fields) in products {
                if img.to.as_ref().is_some_and(|t| t != p) {
                    continue;
                }
                reaches.insert((s.clone(), p.clone()));
                for g in fields {
                    let from = img
                        .to
                        .as_ref()
                        .and_then(|_| img.given.iter().find(|(x, _)| x == g))
                        .and_then(|(_, f)| f.clone());
                    given
                        .entry((s.clone(), p.clone(), g.clone()))
                        .or_default()
                        .push(from);
                }
            }
        }
    }
    (reaches, given)
}

/// Returns the name of constructor `d` with the names of its fields.
pub(super) fn ctor_fields(d: &ast::CtorDef) -> (String, Vec<String>) {
    (
        d.name.text.clone(),
        d.record
            .iter()
            .flatten()
            .map(|f| f.name.text.clone())
            .collect(),
    )
}

/// Returns where a merge without routing gives each field of the merged constructor `product`,
/// with the names of its fields, for rows of `sources`, given what it carries by name.
pub(super) fn by_name(
    carries: &[(String, String, String, String)],
    sources: &[String],
    (product, fields): &(String, Vec<String>),
) -> (Reaches, Given) {
    let mut reaches = Reaches::new();
    let mut given = Given::new();
    for s in sources {
        reaches.insert((s.clone(), product.clone()));
        for g in fields {
            let from = carries
                .iter()
                .find(|(x, _, p, y)| x == s && p == product && y == g)
                .map(|(_, f, _, _)| f.clone());
            given.insert((s.clone(), product.clone(), g.clone()), vec![from]);
        }
    }
    (reaches, given)
}

impl Cx<'_> {
    /// Record how the operator carries grants on constructors `sources` of type `def` to
    /// `products`, for [`Source::carry_grants`] to apply once the products have ids.
    pub(super) fn plan_carry(
        &mut self,
        what: Structural,
        schema: &Schema,
        (def, ty): (TypeId, &QName),
        (sources, products): (&[String], &[String]),
        (reaches, given, discriminators, matched): (&Reaches, &Given, &[String], &[String]),
    ) {
        let td = schema.ty(def);
        let ids = &schema.ids;
        let ctor = |name: &str| {
            td.ctor_index(name)
                .and_then(|c| ids.ctors.get(&(def, c)).copied())
        };
        let field = |s: &str, f: &str| {
            let c = td.ctor_index(s);
            ids.fields
                .get(&(def, c, f.to_string()))
                .or_else(|| ids.fields.get(&(def, None, f.to_string())))
                .copied()
        };
        let mut relations = Vec::new();
        let mut sets = BTreeMap::new();
        for (rel, _, name) in Self::relations_of(schema, def) {
            let Some(&id) = ids.relations.get(&rel) else {
                continue;
            };
            relations.push((id, name));
            for set in &schema.relation(rel).sets {
                if let Some(&sid) = ids.sets.get(&(rel, set.name.clone())) {
                    let held = set
                        .ctors
                        .iter()
                        .filter(|&&c| sources.contains(&td.ctors()[c as usize].name))
                        .filter_map(|&c| ids.ctors.get(&(def, c)).copied())
                        .collect();
                    sets.insert(sid, held);
                }
            }
        }
        self.src.carry = Some(Carry {
            what,
            span: self.span,
            ty: format!("{}::{}", self.module_of(ty), td.name),
            relations,
            sets,
            sources: sources
                .iter()
                .filter_map(|s| Some((ctor(s)?, s.clone())))
                .collect(),
            products: products.to_vec(),
            reaches: reaches
                .iter()
                .filter_map(|(s, p)| Some((ctor(s)?, p.clone())))
                .collect(),
            given: given
                .iter()
                .filter_map(|((s, p, g), from)| {
                    let entries = from
                        .iter()
                        .map(|f| f.as_ref().and_then(|f| field(s, f)))
                        .collect();
                    Some(((ctor(s)?, p.clone(), g.clone()), entries))
                })
                .collect(),
            discriminators: discriminators.to_vec(),
            matched: sources
                .iter()
                .flat_map(|s| matched.iter().filter_map(|f| field(s, f)))
                .collect(),
        });
    }
}

/// What carrying one grant target gives: the products it now covers, the product fields it
/// now covers, and the products it could not be carried to.
#[derive(Default)]
struct Carried {
    ctors: Vec<u64>,
    fields: Vec<u64>,
    unstated: Vec<(u64, String)>,
}

impl Carry {
    /// Returns what target `t`, with fields `fields` for a `read` or `update` and whether it
    /// may cover a field that tells the sources apart, is carried to: a merge's discriminator
    /// or a nesting field of several sources.
    ///
    /// A product receives the grant when the target covers every source whose rows go to it,
    /// and every such source the same fields of it: those the routing gives only by carrying
    /// whole a covered field. When the target covers some of those sources but not all, or
    /// them with different fields, or for a nest some but not all of a source's fields, the
    /// product is unstated.
    fn target(
        &self,
        t: &GrantTarget,
        fields: Option<(&GrantFields, bool)>,
        products: &[(String, u64)],
        field_id: &dyn Fn(&str, &str) -> Option<u64>,
    ) -> Carried {
        let mut out = Carried::default();
        let covered = self.covered(t);
        if covered.is_empty() {
            return out;
        }
        for (p, pid) in products {
            let reach: BTreeSet<u64> = self
                .reaches
                .iter()
                .filter(|(_, q)| q == p)
                .map(|(s, _)| *s)
                .collect();
            let cov: BTreeSet<u64> = reach.intersection(&covered).copied().collect();
            if cov.is_empty() {
                continue;
            }
            if cov != reach {
                out.unstated.push((*pid, p.clone()));
                continue;
            }
            if let Some((fs, moves)) = fields {
                if self.what == Structural::Nest && self.partly(&cov, p, fs) {
                    out.unstated.push((*pid, p.clone()));
                    continue;
                }
                let image = |s: u64| -> BTreeSet<&str> {
                    self.given
                        .iter()
                        .filter(|((x, q, _), from)| {
                            *x == s
                                && q == p
                                && from.iter().all(|f| f.is_some_and(|f| fs.ids.contains(&f)))
                        })
                        .map(|((_, _, g), _)| g.as_str())
                        .collect()
                };
                let mut images = cov.iter().map(|&s| image(s));
                let first = images.next().unwrap_or_default();
                if images.any(|i| i != first) {
                    out.unstated.push((*pid, p.clone()));
                    continue;
                }
                let nested = moves || self.what != Structural::Nest;
                out.fields.extend(
                    first
                        .iter()
                        .copied()
                        .filter(|_| nested)
                        .chain(
                            self.discriminators
                                .iter()
                                .filter(|_| moves)
                                .map(String::as_str),
                        )
                        .filter_map(|g| field_id(p, g)),
                );
            }
            out.ctors.push(*pid);
        }
        out
    }

    /// Returns the sources target `t` covers.
    fn covered(&self, t: &GrantTarget) -> BTreeSet<u64> {
        if !self.relations.iter().any(|(r, _)| *r == t.relation) {
            return BTreeSet::new();
        }
        match &t.sel {
            GrantSel::Whole => self.sources.iter().map(|(s, _)| *s).collect(),
            GrantSel::Ctors(cs) => self
                .sources
                .iter()
                .map(|(s, _)| *s)
                .filter(|s| cs.contains(s))
                .collect(),
            GrantSel::Set(set) => self.sets.get(set).cloned().unwrap_or_default(),
        }
    }

    /// Returns whether fields `fs` cover some but not all of the fields a field of product `p`
    /// is given from, for a source of `sources`.
    fn partly(&self, sources: &BTreeSet<u64>, p: &str, fs: &GrantFields) -> bool {
        self.given.iter().any(|((s, q, _), from)| {
            let n = from
                .iter()
                .filter(|f| f.is_some_and(|f| fs.ids.contains(&f)))
                .count();
            sources.contains(s) && q == p && n > 0 && n < from.len()
        })
    }

    /// Returns whether `grants` give `role` a `retag` between every ordered pair of distinct
    /// sources in relation `relation`.
    fn retags(&self, grants: &[LogGrant], role: &str, relation: u64) -> bool {
        let mut pairs = BTreeSet::new();
        for g in grants.iter().filter(|g| g.role == role) {
            if let Granted::Retag(a, b, except) = &g.privilege
                && a.relation == relation
                && b.relation == relation
            {
                let to = self.covered(b);
                for &x in &self.covered(a) {
                    pairs.extend(
                        to.iter()
                            .filter(|&&y| y != x && !except.contains(&(x, y)))
                            .map(|&y| (x, y)),
                    );
                }
            }
        }
        let n = self.sources.len();
        pairs.len() == n * n.saturating_sub(1)
    }

    /// Returns the ids of the products the rows of constructor `x` go to, or `x` when it is no
    /// source.
    fn images(&self, x: u64, products: &[(String, u64)]) -> Vec<u64> {
        if !self.sources.iter().any(|(s, _)| *s == x) {
            return vec![x];
        }
        products
            .iter()
            .filter(|(p, _)| self.reaches.contains(&(x, p.clone())))
            .map(|&(_, id)| id)
            .collect()
    }

    /// Returns excepted pairs `except` with each source replaced by every product it reaches,
    /// without a pair of a constructor with itself.
    fn excepted(&self, except: &[(u64, u64)], products: &[(String, u64)]) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = except
            .iter()
            .flat_map(|&(x, y)| {
                let to = self.images(y, products);
                self.images(x, products)
                    .into_iter()
                    .flat_map(move |x| to.clone().into_iter().map(move |y| (x, y)))
            })
            .filter(|(x, y)| x != y)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Returns whether a merge or nest replaces sources that excepted pairs `except` tell
    /// apart: some constructor outside the sources is excepted with some of them but not all,
    /// in one direction.
    fn excepts_apart(&self, except: &[(u64, u64)]) -> bool {
        if !matches!(self.what, Structural::Merge | Structural::Nest) {
            return false;
        }
        let sources: BTreeSet<u64> = self.sources.iter().map(|(s, _)| *s).collect();
        let outside: BTreeSet<u64> = except
            .iter()
            .flat_map(|&(x, y)| [x, y])
            .filter(|z| !sources.contains(z))
            .collect();
        outside.iter().any(|&z| {
            let from = sources
                .iter()
                .filter(|&&s| except.contains(&(s, z)))
                .count();
            let to = sources
                .iter()
                .filter(|&&s| except.contains(&(z, s)))
                .count();
            (from != 0 && from != sources.len()) || (to != 0 && to != sources.len())
        })
    }

    /// Returns whether a split or unnest would carry retag `a -> b` to pairs between two of its
    /// products, the source being on both sides.
    fn divides(&self, a: &GrantTarget, b: &GrantTarget) -> bool {
        matches!(self.what, Structural::Split | Structural::Unnest)
            && self.products.len() > 1
            && self
                .covered(a)
                .intersection(&self.covered(b))
                .next()
                .is_some()
    }

    /// Returns whether `grants` give `role` `update` on every field the routing reads of each
    /// source both sides of retag `a -> b` cover.
    fn updates_routing(
        &self,
        grants: &[LogGrant],
        role: &str,
        a: &GrantTarget,
        b: &GrantTarget,
    ) -> bool {
        let both: BTreeSet<u64> = self
            .covered(a)
            .intersection(&self.covered(b))
            .copied()
            .collect();
        let mut fields = BTreeSet::new();
        for g in grants.iter().filter(|g| g.role == role) {
            if let Granted::Update(t, fs) = &g.privilege
                && t.relation == a.relation
                && both.is_subset(&self.covered(t))
            {
                fields.extend(fs.ids.iter().copied());
            }
        }
        self.matched.is_subset(&fields)
    }

    fn unstated(&self, role: &str, relation: u64, kind: Kind, product: (u64, String)) -> Unstated {
        let sources: Vec<String> = self.sources.iter().map(|(_, s)| format!("`{s}`")).collect();
        let rel = self
            .relations
            .iter()
            .find(|(r, _)| *r == relation)
            .map_or("", |(_, n)| n.as_str());
        Unstated {
            role: role.to_string(),
            relation,
            kind,
            product: product.0,
            span: self.span,
            message: format!(
                "the {} of {} into `{}` cannot carry the `{k}` grant of `{role}` on `{rel}`, which does not cover each of them alike; grant `{k}` on `{rel}.{}` to `{role}` in the same migration",
                self.what.name(),
                sources.join(", "),
                product.1,
                product.1,
                k = kind.name(),
            ),
        }
    }
}

impl Source {
    /// Carry the log grants across operator `op` of a migration of `module`, applied to `before`
    /// to give this source, and drop what they name that no longer exists.
    pub(super) fn carry_grants(&mut self, before: &Source, module: &str, op: &OperatorKind) {
        let plan = self.carry.take();
        if self.grants.is_empty() {
            return;
        }
        if let Some(plan) = plan {
            self.carry_plan(&plan);
        }
        let q = |n: &QName| {
            n.module
                .as_ref()
                .map_or(module.to_string(), |m| m.text.clone())
        };
        match op {
            OperatorKind::AddConstructor { path, .. } => {
                self.widen(&q(&path.ty), &path.ty.name.text, &path.ctor.text);
            }
            OperatorKind::MoveField {
                path,
                sel: Some(sel),
                ..
            } if path.ctor.is_none() => self.spread(before, &q(&path.ty), path, sel),
            OperatorKind::MoveField {
                path,
                sel: None,
                fill: Some(fill),
                ..
            } => self.lift(&q(&path.ty), path, fill),
            _ => {}
        }
        self.prune_grants();
    }

    fn carry_plan(&mut self, plan: &Carry) {
        let ids = &self.ids;
        let products: Vec<(String, u64)> = plan
            .products
            .iter()
            .filter_map(|p| Some((p.clone(), *ids.get(&format!("ctor {}.{p}", plan.ty))?)))
            .collect();
        let field_id = |p: &str, g: &str| ids.get(&format!("field {}.{p}.{g}", plan.ty)).copied();
        let mut unstated: Vec<Unstated> = Vec::new();
        let held = self.grants.clone();
        let mut grants = std::mem::take(&mut self.grants);
        for g in &mut grants {
            let kind = Kind::of(&g.privilege);
            let role = g.role.clone();
            let mut carry = |t: &mut GrantTarget, fields: Option<&mut GrantFields>| {
                let moves = kind == Kind::Read
                    || (kind == Kind::Update && plan.retags(&held, &role, t.relation));
                let out = plan.target(
                    t,
                    fields.as_deref().map(|fs| (fs, moves)),
                    &products,
                    &field_id,
                );
                if let GrantSel::Ctors(cs) = &mut t.sel {
                    cs.extend(out.ctors);
                    cs.sort_unstable();
                    cs.dedup();
                }
                if let Some(fs) = fields {
                    fs.ids.extend(out.fields);
                    fs.ids.sort_unstable();
                    fs.ids.dedup();
                }
                for p in out.unstated {
                    let u = plan.unstated(&role, t.relation, kind, p);
                    if !unstated.iter().any(|x| x.same(&u)) {
                        unstated.push(u);
                    }
                }
            };
            match &mut g.privilege {
                Granted::Read(t, fs) | Granted::Update(t, fs) => carry(t, Some(fs)),
                Granted::Insert(t) | Granted::Delete(t) => carry(t, None),
                Granted::Retag(a, b, except) => {
                    let divided = plan.divides(a, b) && !plan.updates_routing(&held, &role, a, b);
                    let differs = plan.excepts_apart(except);
                    carry(a, None);
                    carry(b, None);
                    *except = plan.excepted(except, &products);
                    if differs {
                        for (p, id) in &products {
                            let u = plan.unstated(&role, a.relation, kind, (*id, p.clone()));
                            if !unstated.iter().any(|x| x.same(&u)) {
                                unstated.push(u);
                            }
                        }
                    }
                    if divided {
                        for (_, x) in &products {
                            except.extend(
                                products
                                    .iter()
                                    .filter(|(_, y)| y != x)
                                    .map(|&(_, y)| (*x, y)),
                            );
                        }
                        except.sort_unstable();
                        except.dedup();
                    }
                }
                Granted::CallFn(_) => {}
            }
        }
        self.grants = grants;
        for u in unstated {
            if !self.unstated.iter().any(|x| x.same(&u)) {
                self.unstated.push(u);
            }
        }
    }

    /// Give the fields of new constructor `c` of type `t` of module `m` to each grant without a
    /// field list on a whole relation variable over `t` or on a named set `c` joins, and to each
    /// grant with a list on a set `c` joins the fields of `c` it lists by name.
    fn widen(&mut self, m: &str, t: &str, c: &str) {
        let Ok(schema) = self.schema() else {
            return;
        };
        let Some(def) = schema
            .module_named(m)
            .and_then(|mid| schema.module(mid).types.get(t).copied())
        else {
            return;
        };
        let Some(ci) = schema.ty(def).ctor_index(c) else {
            return;
        };
        let rels: BTreeSet<u64> = schema
            .ids
            .relations
            .iter()
            .filter(|(r, _)| schema.relation(**r).elem_def == def)
            .map(|(_, &id)| id)
            .collect();
        let joined: BTreeSet<u64> = schema
            .ids
            .sets
            .iter()
            .filter(|((r, name), _)| {
                schema.relation(*r).elem_def == def
                    && schema
                        .relation(*r)
                        .sets
                        .iter()
                        .any(|x| x.name == *name && x.ctors.contains(&ci))
            })
            .map(|(_, &id)| id)
            .collect();
        let prefix = format!("field {m}::{t}.{c}.");
        let fields: Vec<(&str, u64)> = self
            .ids
            .iter()
            .filter_map(|(p, &id)| Some((p.strip_prefix(&prefix)?, id)))
            .collect();
        let name = |id: u64| {
            self.ids
                .iter()
                .find(|(p, x)| **x == id && p.starts_with(IdKind::Field.prefix()))
                .and_then(|(p, _)| p.rsplit('.').next())
        };
        let mut gained: Vec<(usize, Vec<u64>)> = Vec::new();
        for (k, g) in self.grants.iter().enumerate() {
            let (Granted::Read(t, fs) | Granted::Update(t, fs)) = &g.privilege else {
                continue;
            };
            let open = match t.sel {
                GrantSel::Whole => rels.contains(&t.relation),
                GrantSel::Set(s) => joined.contains(&s),
                GrantSel::Ctors(_) => false,
            };
            if !open || (fs.listed && t.sel == GrantSel::Whole) {
                continue;
            }
            let listed: BTreeSet<&str> = fs.ids.iter().filter_map(|&f| name(f)).collect();
            gained.push((
                k,
                fields
                    .iter()
                    .filter(|(n, _)| !fs.listed || listed.contains(n))
                    .map(|&(_, id)| id)
                    .collect(),
            ));
        }
        for (k, ids) in gained {
            if let Granted::Read(_, fs) | Granted::Update(_, fs) = &mut self.grants[k].privilege {
                fs.ids.extend(ids);
                fs.ids.sort_unstable();
                fs.ids.dedup();
            }
        }
    }

    /// Give each grant on header field `path` of type `m::T` the field a move down made of it
    /// in each constructor of `sel` the grant covers, in place of the header field.
    fn spread(&mut self, before: &Source, m: &str, path: &ast::FieldPath, sel: &CtorSel) {
        let t = &path.ty.name.text;
        let f = &path.field.text;
        let Some(&old) = before.ids.get(&format!("field {m}::{t}.{f}")) else {
            return;
        };
        let Ok(schema) = self.schema() else {
            return;
        };
        let moved: Vec<(u64, u64)> = sel_idents(sel)
            .iter()
            .filter_map(|c| {
                Some((
                    *self.ids.get(&format!("ctor {m}::{t}.{}", c.text))?,
                    *self.ids.get(&format!("field {m}::{t}.{}.{f}", c.text))?,
                ))
            })
            .collect();
        for g in &mut self.grants {
            if let Granted::Read(t, fs) | Granted::Update(t, fs) = &mut g.privilege
                && fs.ids.contains(&old)
            {
                let covered = covered_ctors(&schema, t);
                fs.ids.retain(|&x| x != old);
                fs.ids.extend(
                    moved
                        .iter()
                        .filter(|(c, _)| covered.contains(c))
                        .map(|&(_, x)| x),
                );
                fs.ids.sort_unstable();
                fs.ids.dedup();
            }
        }
    }

    /// Take the header field a move up made of field `path` of type `m::T` from each grant
    /// covering a constructor other than the field's whose value `fill` does not carry whole
    /// from a field the grant covers.
    fn lift(&mut self, m: &str, path: &ast::FieldPath, fill: &Fill) {
        let t = &path.ty.name.text;
        let Some(c) = &path.ctor else {
            return;
        };
        let Some(&moved) = self.ids.get(&format!("field {m}::{t}.{}", path.field.text)) else {
            return;
        };
        let Ok(schema) = self.schema() else {
            return;
        };
        let arms: Vec<(Option<String>, Option<String>)> = match fill {
            Fill::Match(arms) => arms.iter().map(ctor::fill_image).collect(),
            Fill::Expr(_) => vec![(None, None)],
        };
        let ctor_name = |id: u64| {
            self.ids
                .iter()
                .find(|(p, x)| **x == id && p.starts_with(IdKind::Ctor.prefix()))
                .and_then(|(p, _)| p.rsplit('.').next())
                .map(str::to_string)
        };
        let field_id = |d: &str, g: &str| self.ids.get(&format!("field {m}::{t}.{d}.{g}")).copied();
        let mut lost = Vec::new();
        for (k, g) in self.grants.iter().enumerate() {
            let (Granted::Read(target, fs) | Granted::Update(target, fs)) = &g.privilege else {
                continue;
            };
            if !fs.ids.contains(&moved) {
                continue;
            }
            let carried = covered_ctors(&schema, target)
                .into_iter()
                .filter_map(ctor_name)
                .filter(|d| d != &c.text)
                .all(|d| {
                    let reaching: Vec<&Option<String>> = arms
                        .iter()
                        .filter(|(from, _)| from.as_ref().is_none_or(|x| *x == d))
                        .map(|(_, f)| f)
                        .collect();
                    !reaching.is_empty()
                        && reaching.iter().all(|f| {
                            f.as_ref()
                                .and_then(|f| field_id(&d, f))
                                .is_some_and(|x| fs.ids.contains(&x))
                        })
                });
            if !carried {
                lost.push(k);
            }
        }
        for k in lost {
            if let Granted::Read(_, fs) | Granted::Update(_, fs) = &mut self.grants[k].privilege {
                fs.ids.retain(|&x| x != moved);
            }
        }
    }

    /// Drop from each grant the objects that no longer exist, and each grant left naming no
    /// relation, set, function or constructor, and keep one entry per privilege of a role.
    fn prune_grants(&mut self) {
        let kinds = [
            IdKind::Relation,
            IdKind::Ctor,
            IdKind::Set,
            IdKind::Field,
            IdKind::Function,
        ];
        let live: BTreeSet<(&str, u64)> = self
            .ids
            .iter()
            .flat_map(|(p, &id)| {
                kinds
                    .iter()
                    .filter(move |k| p.starts_with(k.prefix()))
                    .map(move |k| (k.prefix(), id))
            })
            .collect();
        let mut kept: Vec<LogGrant> = Vec::new();
        for g in &self.grants {
            let Some(privilege) = g
                .privilege
                .mapped(&mut |k, x| live.contains(&(k.prefix(), x)).then_some(x))
            else {
                continue;
            };
            match kept
                .iter_mut()
                .find(|x| x.role == g.role && x.privilege == privilege)
            {
                Some(x) => x.grant_option |= g.grant_option,
                None => kept.push(LogGrant {
                    role: g.role.clone(),
                    privilege,
                    grant_option: g.grant_option,
                    row: g.row,
                }),
            }
        }
        self.grants = kept;
    }
}

/// Returns the ids of the constructors grant target `t` covers in `schema`.
fn covered_ctors(schema: &Schema, t: &GrantTarget) -> BTreeSet<u64> {
    let Some((&rel, _)) = schema.ids.relations.iter().find(|(_, x)| **x == t.relation) else {
        return BTreeSet::new();
    };
    let def = schema.relation(rel).elem_def;
    let ctor_id = |c: u32| schema.ids.ctors.get(&(def, c)).copied();
    match &t.sel {
        GrantSel::Whole => (0..schema.ty(def).ctors().len() as u32)
            .filter_map(ctor_id)
            .collect(),
        GrantSel::Ctors(cs) => cs.iter().copied().collect(),
        GrantSel::Set(s) => schema
            .ids
            .sets
            .iter()
            .find(|(_, x)| **x == *s)
            .and_then(|((r, name), _)| schema.relation(*r).sets.iter().find(|x| x.name == *name))
            .map(|set| set.ctors.iter().filter_map(|&c| ctor_id(c)).collect())
            .unwrap_or_default(),
    }
}
