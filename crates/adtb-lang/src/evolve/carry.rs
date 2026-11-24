use super::grant::IdKind;
use super::*;

/// How a structural operator carries the grants on the constructors it replaces to the
/// constructors it makes.
#[derive(Clone, Debug)]
pub(super) struct Carry {
    what: Structural,
    span: Span,
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
    pub(super) fn same(&self, o: &Unstated) -> bool {
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

/// What carrying one grant target gives: the products it now covers, the product fields it
/// now covers, and the products it could not be carried to.
#[derive(Default)]
struct Carried {
    ctors: Vec<u64>,
    fields: Vec<u64>,
    unstated: Vec<(u64, String)>,
}

/// What a structural operator replaces and makes, by id, for [`Carry::new`].
pub(super) struct Plan {
    what: Structural,
    span: Span,
    /// The relation variables over the type, by id, with their names.
    relations: Vec<(u64, String)>,
    /// The sources each named set of a relation over the type holds, by set id.
    sets: BTreeMap<u64, BTreeSet<u64>>,
    /// The replaced constructors, by id, with their names.
    sources: Vec<(u64, String)>,
    products: Vec<String>,
    /// The ids of the fields of the sources, by source and field name.
    fields: BTreeMap<(u64, String), u64>,
}

/// The relation variables over a type, by id with their names, and the sources each named set
/// of them holds, by set id.
pub(super) type Over = (Vec<(u64, String)>, BTreeMap<u64, BTreeSet<u64>>);

/// The replaced constructors by id with their names, the products' names, and the ids of the
/// sources' fields by source and field name.
pub(super) type Replacing = (
    Vec<(u64, String)>,
    Vec<String>,
    BTreeMap<(u64, String), u64>,
);

impl Plan {
    /// Returns the plan of structural operator `what` at `span`, over the relation variables
    /// `over`, replacing as `replacing` states.
    pub(super) fn new(what: Structural, span: Span, over: Over, replacing: Replacing) -> Plan {
        let (relations, sets) = over;
        let (sources, products, fields) = replacing;
        Plan {
            what,
            span,
            relations,
            sets,
            sources,
            products,
            fields,
        }
    }

    pub(super) fn products(&self) -> &[String] {
        &self.products
    }
}

impl Carry {
    /// Returns how operator `plan` carries grants, given what its routing reaches and gives by
    /// name, its discriminators and the source fields its routing matches on by name.
    pub(super) fn new(
        plan: Plan,
        (reaches, given): (&Reaches, &Given),
        discriminators: &[String],
        matched: &[String],
    ) -> Carry {
        let ctor = |name: &str| {
            plan.sources
                .iter()
                .find(|(_, n)| n == name)
                .map(|&(id, _)| id)
        };
        let field = |s: &str, f: &str| {
            let c = ctor(s)?;
            plan.fields.get(&(c, f.to_string())).copied()
        };
        Carry {
            what: plan.what,
            span: plan.span,
            relations: plan.relations.clone(),
            sets: plan.sets.clone(),
            sources: plan.sources.clone(),
            products: plan.products.clone(),
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
            matched: plan
                .sources
                .iter()
                .flat_map(|(_, s)| matched.iter().filter_map(|f| field(s, f)))
                .collect(),
        }
    }

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
    fn excepted(&self, except: &[(u64, u64)], products: &[(String, u64)]) -> Ascending<(u64, u64)> {
        except
            .iter()
            .flat_map(|&(x, y)| {
                let to = self.images(y, products);
                self.images(x, products)
                    .into_iter()
                    .flat_map(move |x| to.clone().into_iter().map(move |y| (x, y)))
            })
            .filter(|(x, y)| x != y)
            .collect()
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

    /// Carry `grants` across the operator, whose products are `products` by name and id and
    /// whose product fields `field_id` gives by product and field name. Returns the grants it
    /// could not carry to a product.
    pub(super) fn carry(
        &self,
        grants: &mut [LogGrant],
        products: &[(String, u64)],
        field_id: &dyn Fn(&str, &str) -> Option<u64>,
    ) -> Vec<Unstated> {
        let mut unstated: Vec<Unstated> = Vec::new();
        let held = grants.to_vec();
        let mut note = |u: Unstated| {
            if !unstated.iter().any(|x| x.same(&u)) {
                unstated.push(u);
            }
        };
        for g in grants.iter_mut() {
            let kind = Kind::of(&g.privilege);
            let role = g.role.clone();
            let mut carry = |t: &mut GrantTarget, fields: Option<&mut GrantFields>| {
                let moves = kind == Kind::Read
                    || (kind == Kind::Update && self.retags(&held, &role, t.relation));
                let out = self.target(
                    t,
                    fields.as_deref().map(|fs| (fs, moves)),
                    products,
                    field_id,
                );
                if let GrantSel::Ctors(cs) = &mut t.sel {
                    cs.extend(out.ctors);
                }
                if let Some(fs) = fields {
                    fs.ids.extend(out.fields);
                }
                for p in out.unstated {
                    note(self.unstated(&role, t.relation, kind, p));
                }
            };
            match &mut g.privilege {
                Granted::Read(t, fs) | Granted::Update(t, fs) => carry(t, Some(fs)),
                Granted::Insert(t) | Granted::Delete(t) => carry(t, None),
                Granted::Retag(a, b, except) => {
                    let divided = self.divides(a, b) && !self.updates_routing(&held, &role, a, b);
                    let differs = self.excepts_apart(except);
                    carry(a, None);
                    carry(b, None);
                    *except = self.excepted(except, products);
                    if differs {
                        for (p, id) in products {
                            note(self.unstated(&role, a.relation, kind, (*id, p.clone())));
                        }
                    }
                    if divided {
                        for (_, x) in products {
                            except.extend(
                                products
                                    .iter()
                                    .filter(|(_, y)| y != x)
                                    .map(|&(_, y)| (*x, y)),
                            );
                        }
                    }
                }
                Granted::CallFn(_) => {}
            }
        }
        unstated
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

/// Returns `grants` without each id `live` refuses, each grant left naming no relation, set or
/// function, or no constructor of a list, gone, and grants of one role and privilege joined.
pub(super) fn pruned(grants: &[LogGrant], live: &dyn Fn(IdKind, u64) -> bool) -> Vec<LogGrant> {
    let mut kept: Vec<LogGrant> = Vec::new();
    for g in grants {
        let Some(privilege) = g.privilege.mapped(&mut |k, x| live(k, x).then_some(x)) else {
            continue;
        };
        match kept
            .iter_mut()
            .find(|x| x.role == g.role && x.privilege == privilege)
        {
            Some(x) => match (x.dormant, g.dormant) {
                (true, false) => {
                    x.grant_option = g.grant_option;
                    x.row = g.row;
                    x.dormant = false;
                }
                (false, true) => {}
                _ => x.grant_option |= g.grant_option,
            },
            None => kept.push(LogGrant {
                role: g.role.clone(),
                privilege,
                grant_option: g.grant_option,
                row: g.row,
                dormant: g.dormant,
            }),
        }
    }
    kept
}
