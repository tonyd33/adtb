use super::grant::IdKind;
use super::*;

/// How a structural operator carries the grants on the constructors it replaces to the
/// constructors it makes.
#[derive(Clone, Debug)]
pub(super) struct Carry {
    /// The operator, as a refusal names it.
    what: &'static str,
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
    kind: &'static str,
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

fn kind(p: &Granted) -> &'static str {
    match p {
        Granted::Read(..) => "read",
        Granted::Insert(_) => "insert",
        Granted::Update(..) => "update",
        Granted::Delete(_) => "delete",
        Granted::Retag(..) => "retag",
        Granted::CallFn(_) => "call",
    }
}

fn targets(p: &Granted) -> Vec<&GrantTarget> {
    match p {
        Granted::Read(t, _) | Granted::Update(t, _) | Granted::Insert(t) | Granted::Delete(t) => {
            vec![t]
        }
        Granted::Retag(a, b) => vec![a, b],
        Granted::CallFn(_) => Vec::new(),
    }
}

impl Unstated {
    /// Returns whether privilege `p` granted to `role`, covering constructors `covered`, states
    /// the grant this one lacks.
    pub(super) fn stated_by(&self, role: &str, p: &Granted, covered: &BTreeSet<u64>) -> bool {
        role == self.role
            && kind(p) == self.kind
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
        what: &'static str,
        schema: &Schema,
        (def, ty): (TypeId, &QName),
        (sources, products): (&[String], &[String]),
        (reaches, given): (&Reaches, &Given),
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
    /// Returns what target `t`, with fields `fields` for a `read` or `update`, is carried to.
    ///
    /// A product receives the grant when the target covers every source whose rows go to it,
    /// and every such source the same fields of it: those the routing gives only by carrying
    /// whole a covered field. When the target covers some of those sources but not all, or
    /// them with different fields, the product is unstated.
    fn target(
        &self,
        t: &GrantTarget,
        fields: Option<&GrantFields>,
        products: &[(String, u64)],
        field_id: &dyn Fn(&str, &str) -> Option<u64>,
    ) -> Carried {
        let mut out = Carried::default();
        if !self.relations.iter().any(|(r, _)| *r == t.relation) {
            return out;
        }
        let covered: BTreeSet<u64> = match &t.sel {
            GrantSel::Whole => self.sources.iter().map(|(s, _)| *s).collect(),
            GrantSel::Ctors(cs) => self
                .sources
                .iter()
                .map(|(s, _)| *s)
                .filter(|s| cs.contains(s))
                .collect(),
            GrantSel::Set(set) => self.sets.get(set).cloned().unwrap_or_default(),
        };
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
            if let Some(fs) = fields {
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
                out.fields
                    .extend(first.iter().filter_map(|g| field_id(p, g)));
            }
            out.ctors.push(*pid);
        }
        out
    }

    fn unstated(
        &self,
        role: &str,
        relation: u64,
        kind: &'static str,
        product: (u64, String),
    ) -> Unstated {
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
                "the {} of {} into `{}` cannot carry the `{kind}` grant of `{role}` on `{rel}`, which does not cover each of them with the same fields; grant `{}` to `{role}` in the same migration",
                self.what,
                sources.join(", "),
                product.1,
                product.1,
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
        let mut unstated = Vec::new();
        let mut grants = std::mem::take(&mut self.grants);
        for g in &mut grants {
            let kind = kind(&g.privilege);
            let mut carry = |t: &mut GrantTarget, fields: Option<&mut GrantFields>| {
                let out = plan.target(t, fields.as_deref(), &products, &field_id);
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
                    unstated.push(plan.unstated(&g.role, t.relation, kind, p));
                }
            };
            match &mut g.privilege {
                Granted::Read(t, fs) | Granted::Update(t, fs) => carry(t, Some(fs)),
                Granted::Insert(t) | Granted::Delete(t) => carry(t, None),
                Granted::Retag(a, b) => {
                    carry(a, None);
                    carry(b, None);
                }
                Granted::CallFn(_) => {}
            }
        }
        self.grants = grants;
        self.unstated.extend(unstated);
    }

    /// Give each grant without a field list on a whole relation variable over type `t` of module
    /// `m` the fields of its new constructor `c`.
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
        let rels: BTreeSet<u64> = schema
            .ids
            .relations
            .iter()
            .filter(|(r, _)| schema.relation(**r).elem_def == def)
            .map(|(_, &id)| id)
            .collect();
        let prefix = format!("field {m}::{t}.{c}.");
        let fields: Vec<u64> = self
            .ids
            .iter()
            .filter(|(p, _)| p.starts_with(&prefix))
            .map(|(_, &id)| id)
            .collect();
        for g in &mut self.grants {
            if let Granted::Read(t, fs) | Granted::Update(t, fs) = &mut g.privilege
                && t.sel == GrantSel::Whole
                && !fs.listed
                && rels.contains(&t.relation)
            {
                fs.ids.extend(&fields);
                fs.ids.sort_unstable();
                fs.ids.dedup();
            }
        }
    }

    /// Give each grant on header field `path` of type `m::T` the field of each constructor of
    /// `sel` that a move down made of it.
    fn spread(&mut self, before: &Source, m: &str, path: &ast::FieldPath, sel: &CtorSel) {
        let t = &path.ty.name.text;
        let f = &path.field.text;
        let Some(&old) = before.ids.get(&format!("field {m}::{t}.{f}")) else {
            return;
        };
        let moved: Vec<u64> = sel_idents(sel)
            .iter()
            .filter_map(|c| {
                self.ids
                    .get(&format!("field {m}::{t}.{}.{f}", c.text))
                    .copied()
            })
            .collect();
        for g in &mut self.grants {
            if let Granted::Read(_, fs) | Granted::Update(_, fs) = &mut g.privilege
                && fs.ids.contains(&old)
            {
                fs.ids.extend(&moved);
                fs.ids.sort_unstable();
                fs.ids.dedup();
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
                }),
            }
        }
        self.grants = kept;
    }
}
