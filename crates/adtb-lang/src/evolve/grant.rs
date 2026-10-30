use super::*;

const USER_IDS: &str = "every object of a user module has a catalog id";

/// A privilege a log `grant` gives a role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogGrant {
    pub role: String,
    pub privilege: Granted,
    pub grant_option: bool,
    /// The id of the `catalog::grants` row of this grant, once the catalog gives it one. An
    /// operator that keeps the grant keeps its id; a grant an operator adds has none.
    pub row: Option<i64>,
    /// Whether the grant allows nothing because its role did not exist when the catalog wrote
    /// its row. A grant operator naming the grant's privilege clears it.
    pub dormant: bool,
}

/// A privilege on schema objects, each named by its catalog id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Granted {
    Read(GrantTarget, GrantFields),
    Insert(GrantTarget),
    Update(GrantTarget, GrantFields),
    Delete(GrantTarget),
    /// A retag between the two targets, except between each pair of constructor ids of the
    /// third, held in ascending order.
    Retag(GrantTarget, GrantTarget, Vec<(u64, u64)>),
    CallFn(u64),
}

/// A relation variable, or the constructors of it a grant selects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantTarget {
    pub relation: u64,
    pub sel: GrantSel,
}

impl GrantTarget {
    /// Returns the ids of the constructors of the target relation that this target covers in
    /// `schema`. A constructor id `schema` lacks, or one of another type, covers nothing.
    pub fn ctors(&self, schema: &Schema) -> BTreeSet<u64> {
        let Some(rel) = schema.ids.relation(self.relation) else {
            return BTreeSet::new();
        };
        let r = schema.relation(rel);
        schema
            .ids
            .ctors
            .iter()
            .filter(|((def, _), _)| *def == r.elem_def)
            .filter(|((_, c), id)| match &self.sel {
                GrantSel::Whole => true,
                GrantSel::Ctors(ids) => ids.contains(id),
                GrantSel::Set(s) => r.sets.iter().any(|set| {
                    schema.ids.sets.get(&(rel, set.name.clone())) == Some(s)
                        && set.ctors.contains(c)
                }),
            })
            .map(|(_, &id)| id)
            .collect()
    }
}

/// The constructors a grant selects. `Ctors` holds its ids in ascending order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantSel {
    Whole,
    Ctors(Vec<u64>),
    Set(u64),
}

/// The fields a `read` or `update` grant covers, by catalog id in ascending order. `listed` is
/// false when the grant states no field list: `ids` then holds the fields its constructors had
/// when it was applied, and for a grant on the whole relation variable also every field of each
/// constructor added since.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantFields {
    pub ids: Vec<u64>,
    pub listed: bool,
}

/// The kind of object a catalog id of a grant names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IdKind {
    Relation,
    Ctor,
    Set,
    Field,
    Function,
}

impl IdKind {
    /// Returns the prefix of the paths of objects of this kind in [`Source::ids`].
    pub(super) fn prefix(self) -> &'static str {
        match self {
            IdKind::Relation => "relation ",
            IdKind::Ctor => "ctor ",
            IdKind::Set => "set ",
            IdKind::Field => "field ",
            IdKind::Function => "fn ",
        }
    }
}

impl GrantTarget {
    fn mapped(&self, f: &mut dyn FnMut(IdKind, u64) -> Option<u64>) -> Option<GrantTarget> {
        let sel = match &self.sel {
            GrantSel::Whole => GrantSel::Whole,
            GrantSel::Set(s) => GrantSel::Set(f(IdKind::Set, *s)?),
            GrantSel::Ctors(cs) => {
                let mut cs: Vec<u64> = cs.iter().filter_map(|&c| f(IdKind::Ctor, c)).collect();
                cs.sort_unstable();
                cs.dedup();
                if cs.is_empty() {
                    return None;
                }
                GrantSel::Ctors(cs)
            }
        };
        Some(GrantTarget {
            relation: f(IdKind::Relation, self.relation)?,
            sel,
        })
    }
}

impl GrantFields {
    fn mapped(&self, f: &mut dyn FnMut(IdKind, u64) -> Option<u64>) -> GrantFields {
        let mut ids: Vec<u64> = self
            .ids
            .iter()
            .filter_map(|&x| f(IdKind::Field, x))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        GrantFields {
            ids,
            listed: self.listed,
        }
    }
}

impl Granted {
    /// Returns the privilege with each id `f` maps, without each constructor or field it maps
    /// to `None`; `None` when it maps the relation, set or function to `None`, or leaves a
    /// constructor list empty.
    pub(super) fn mapped(&self, f: &mut dyn FnMut(IdKind, u64) -> Option<u64>) -> Option<Granted> {
        Some(match self {
            Granted::Read(t, fs) => Granted::Read(t.mapped(f)?, fs.mapped(f)),
            Granted::Update(t, fs) => Granted::Update(t.mapped(f)?, fs.mapped(f)),
            Granted::Insert(t) => Granted::Insert(t.mapped(f)?),
            Granted::Delete(t) => Granted::Delete(t.mapped(f)?),
            Granted::Retag(a, b, except) => {
                let mut pairs: Vec<(u64, u64)> = except
                    .iter()
                    .filter_map(|&(x, y)| Some((f(IdKind::Ctor, x)?, f(IdKind::Ctor, y)?)))
                    .collect();
                pairs.sort_unstable();
                pairs.dedup();
                Granted::Retag(a.mapped(f)?, b.mapped(f)?, pairs)
            }
            Granted::CallFn(x) => Granted::CallFn(f(IdKind::Function, *x)?),
        })
    }
}

/// Returns `privilege` with each id that `now` does not hold mapped through its path in `then`
/// to the id `now` gives that path.
pub(super) fn translated(
    privilege: &Granted,
    then: &BTreeMap<String, u64>,
    now: &BTreeMap<String, u64>,
) -> Granted {
    let live = |kind: IdKind, x: u64| {
        now.iter()
            .any(|(p, &id)| id == x && p.starts_with(kind.prefix()))
    };
    privilege
        .mapped(&mut |kind, x| {
            if live(kind, x) {
                return Some(x);
            }
            Some(
                then.iter()
                    .find(|(p, id)| **id == x && p.starts_with(kind.prefix()))
                    .and_then(|(p, _)| now.get(p).copied())
                    .unwrap_or(x),
            )
        })
        .expect("a translation maps every id")
}

/// What a role holds of a privilege: its place among the grants, its grant option and its
/// dormancy, `None` when it holds nothing.
#[derive(Clone, Debug)]
pub(super) struct HeldGrant {
    pub(super) role: String,
    pub(super) privilege: Granted,
    pub(super) held: Option<(usize, bool, bool)>,
}

impl Source {
    /// Set what each role of `back` holds of its privilege to what it states, putting a grant
    /// it lacks back at its place. Each privilege names objects by the ids `then` gives their
    /// paths; an object this source holds again under a new id is named by that id.
    ///
    /// Preconditions:
    /// - The privileges of `back` with a place are in ascending order of it.
    pub(super) fn hold(&mut self, back: Vec<HeldGrant>, then: &BTreeMap<String, u64>) {
        let (gone, kept): (Vec<HeldGrant>, Vec<HeldGrant>) =
            back.into_iter().partition(|h| h.held.is_none());
        for h in gone.into_iter().chain(kept) {
            let privilege = translated(&h.privilege, then, &self.ids);
            let at = self
                .grants
                .iter()
                .position(|x| x.role == h.role && x.privilege == privilege);
            match (at, h.held) {
                (Some(i), Some((_, grant_option, dormant))) => {
                    self.grants[i].grant_option = grant_option;
                    self.grants[i].dormant = dormant;
                }
                (Some(i), None) => {
                    self.grants.remove(i);
                }
                (None, Some((i, grant_option, dormant))) => self.grants.insert(
                    i.min(self.grants.len()),
                    LogGrant {
                        role: h.role,
                        privilege,
                        grant_option,
                        row: None,
                        dormant,
                    },
                ),
                (None, None) => {}
            }
        }
    }
}

impl Cx<'_> {
    /// Add each privilege to the role's log grants, keeping one entry per privilege and making
    /// it no longer dormant, with the grant option this grant states when it was dormant, and
    /// settle each grant of the role that a structural operator of this migration could not
    /// carry to a constructor the privilege covers.
    pub(super) fn grant(&mut self, g: &ast::Grant) -> R<()> {
        for Stated {
            privilege, covered, ..
        } in self.granted(&g.privileges, Statement::Grant)?
        {
            self.src
                .unstated
                .retain(|u| !u.stated_by(&g.to.text, &privilege, &covered));
            match self
                .src
                .grants
                .iter_mut()
                .find(|x| x.role == g.to.text && x.privilege == privilege)
            {
                Some(held) if held.dormant => {
                    held.grant_option = g.grant_option;
                    held.dormant = false;
                }
                Some(held) => held.grant_option |= g.grant_option,
                None => self.src.grants.push(LogGrant {
                    role: g.to.text.clone(),
                    privilege,
                    grant_option: g.grant_option,
                    row: None,
                    dormant: false,
                }),
            }
        }
        Ok(())
    }

    /// Delete from the role's log grants each one a privilege of `r` matches.
    ///
    /// Fail, naming the revoke, when a privilege matches none of them.
    pub(super) fn revoke(&mut self, r: &ast::Revoke) -> R<()> {
        let gone = self.granted(&r.privileges, Statement::Revoke)?;
        for stated in &gone {
            let held = self
                .src
                .grants
                .iter()
                .any(|x| x.role == r.from.text && stated.revokes(&x.privilege));
            if !held {
                return self.err_at(
                    r.span,
                    format!(
                        "`{}` matches no grant `{}` holds",
                        self.slice(r.span),
                        r.from.text
                    ),
                );
            }
        }
        self.src
            .grants
            .retain(|x| x.role != r.from.text || !gone.iter().any(|p| p.revokes(&x.privilege)));
        Ok(())
    }

    /// Resolve the privileges of a log grant or revoke to the catalog ids of what they name.
    fn granted(&self, privileges: &[ast::Privilege], statement: Statement) -> R<Vec<Stated>> {
        let schema = self.schema()?;
        let vis = crate::resolve::Visible::module(
            schema.module_named(&self.module).unwrap_or(schema::PRELUDE),
        );
        let mut r = Resolving {
            schema: &schema,
            vis: &vis,
            statement,
            system: false,
            span: self.span,
            refs: Vec::new(),
        };
        let out = privileges
            .iter()
            .map(|p| r.privilege(p))
            .collect::<R<Vec<_>>>()?;
        self.note(&schema, &r.refs);
        Ok(out)
    }
}

/// Which statement a privilege is written in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Statement {
    Grant,
    Revoke,
}

/// A privilege of a statement, resolved to catalog ids.
#[derive(Clone, Debug)]
pub struct Stated {
    privilege: Granted,
    /// The ids of the constructors its targets cover.
    covered: BTreeSet<u64>,
    /// Whether it is a `read` or `update` that writes no field list.
    unlisted: bool,
}

impl Stated {
    /// Returns whether a revoke of this privilege matches `held`: the same privilege and target
    /// text, except that a revoke writing no field list matches every frozen field set and a
    /// retag revoke writing no `except` matches whatever pairs are excepted.
    pub fn revokes(&self, held: &Granted) -> bool {
        match (&self.privilege, held) {
            (Granted::Read(a, _), Granted::Read(b, fs))
            | (Granted::Update(a, _), Granted::Update(b, fs))
                if self.unlisted =>
            {
                a == b && !fs.listed
            }
            (Granted::Retag(a, b, written), Granted::Retag(x, y, _)) if written.is_empty() => {
                a == x && b == y
            }
            (written, _) => written == held,
        }
    }
}

struct Resolving<'a> {
    schema: &'a Schema,
    vis: &'a crate::resolve::Visible,
    statement: Statement,
    /// Whether the statement is a deployment statement on system relations.
    system: bool,
    span: Span,
    refs: Vec<(Span, Ref)>,
}

impl Resolving<'_> {
    fn target(&mut self, r: &ast::RelationRef) -> R<(RelId, GrantTarget, CtorSet)> {
        let schema = self.schema;
        let rel = crate::resolve::lookup_relation(schema, self.vis, &r.relation)
            .map_err(|e| vec![Diag::new(r.span, e)])?;
        let rd = schema.relation(rel);
        if schema.module(rd.module).engine != self.system {
            let why = if self.system {
                "a grant on a relation variable of the schema is a schema operator; write it in a migration"
            } else {
                "a grant on a `catalog::` or `sys::` relation is deployment state, an ad hoc statement, never an operator in a migration"
            };
            return Err(vec![Diag::new(r.span, why)]);
        }
        let def = rd.elem_def;
        let ids = &schema.ids;
        self.refs.push((r.relation.name.span, Ref::Relation(rel)));
        let (sel, ctors) = match &r.sel {
            None => (
                GrantSel::Whole,
                (0..schema.ty(def).ctors().len() as u32).collect(),
            ),
            Some(sel) => {
                let ctors = schema
                    .relation_sel(rel, sel)
                    .map_err(|e| vec![Diag::new(r.span, e)])?;
                for x in sel_idents(sel) {
                    self.refs.push(match schema.ty(def).ctor_index(&x.text) {
                        Some(c) => (x.span, Ref::Ctor(def, c)),
                        None => (x.span, Ref::Set(rel, x.text.clone())),
                    });
                }
                match sel {
                    CtorSel::One(x) if schema.ty(def).ctor_index(&x.text).is_none() => (
                        GrantSel::Set(*ids.sets.get(&(rel, x.text.clone())).expect(USER_IDS)),
                        ctors,
                    ),
                    _ => {
                        let mut out: Vec<u64> = ctors
                            .iter()
                            .map(|&c| *ids.ctors.get(&(def, c)).expect(USER_IDS))
                            .collect();
                        out.sort_unstable();
                        (GrantSel::Ctors(out), ctors)
                    }
                }
            }
        };
        let relation = *ids.relations.get(&rel).expect(USER_IDS);
        Ok((rel, GrantTarget { relation, sel }, ctors))
    }

    /// Returns the fields of a `read` or `update`: without a list, those its constructors have
    /// now; a `frozen` list states those fields itself.
    fn fields(
        &self,
        rel: RelId,
        ctors: &CtorSet,
        list: Option<&ast::GrantFieldList>,
    ) -> R<GrantFields> {
        let schema = self.schema;
        let def = schema.relation(rel).elem_def;
        let ids = &schema.ids;
        let Some(list) = list else {
            let mut out: Vec<u64> = ids
                .fields
                .iter()
                .filter(|((d, c, _), _)| *d == def && c.is_none_or(|c| ctors.contains(&c)))
                .map(|(_, &id)| id)
                .collect();
            out.sort_unstable();
            return Ok(GrantFields {
                ids: out,
                listed: false,
            });
        };
        let rd = schema.relation(rel);
        let mut out = Vec::new();
        for f in &list.fields {
            let refuse = |msg: String| Err(vec![Diag::new(f.name.span, msg)]);
            let name = &f.name.text;
            if let Some(c) = &f.ctor {
                let Some(k) = schema.ty(def).ctor_index(&c.text) else {
                    return refuse(format!("`{}` has no constructor `{}`", rd.name, c.text));
                };
                if self.statement == Statement::Grant && !ctors.contains(&k) {
                    return refuse(format!(
                        "`{}` is not a constructor this grant covers",
                        c.text
                    ));
                }
                let Some(&id) = ids.fields.get(&(def, Some(k), name.clone())) else {
                    return refuse(format!(
                        "`{}.{}` is no field of `{}`",
                        c.text, name, rd.name
                    ));
                };
                out.push(id);
            } else if let Some(&id) = ids.fields.get(&(def, None, name.clone())) {
                out.push(id);
            } else {
                let found: Vec<u64> = ctors
                    .iter()
                    .filter_map(|&c| ids.fields.get(&(def, Some(c), name.clone())).copied())
                    .collect();
                if found.is_empty() {
                    return refuse(format!("`{}` has no field `{name}`", rd.name));
                }
                if found.len() < ctors.len() {
                    return refuse(format!(
                        "`{name}` is not a field every constructor of the set declares; write it as `C.{name}`"
                    ));
                }
                out.extend(found);
            }
        }
        out.sort_unstable();
        out.dedup();
        Ok(GrantFields {
            ids: out,
            listed: !list.frozen,
        })
    }

    fn excepted(&self, from: RelId, to: RelId, e: Option<&ast::Excepted>) -> R<Vec<(u64, u64)>> {
        let schema = self.schema;
        let ctor = |rel: RelId, c: &ast::Ident| -> R<u64> {
            let def = schema.relation(rel).elem_def;
            schema
                .ty(def)
                .ctor_index(&c.text)
                .and_then(|k| schema.ids.ctors.get(&(def, k)).copied())
                .ok_or_else(|| {
                    let name = &schema.relation(rel).name;
                    vec![Diag::new(
                        c.span,
                        format!("`{name}` has no constructor `{}`", c.text),
                    )]
                })
        };
        let mut pairs = Vec::new();
        for (x, y) in e.iter().flat_map(|e| &e.pairs) {
            pairs.push((ctor(from, x)?, ctor(to, y)?));
        }
        pairs.sort_unstable();
        pairs.dedup();
        Ok(pairs)
    }

    fn privilege(&mut self, p: &ast::Privilege) -> R<Stated> {
        let schema = self.schema;
        let ids = &schema.ids;
        let covering = |rel: RelId, ctors: &CtorSet, out: &mut BTreeSet<u64>| {
            let def = schema.relation(rel).elem_def;
            out.extend(
                ctors
                    .iter()
                    .map(|&c| *ids.ctors.get(&(def, c)).expect(USER_IDS)),
            );
        };
        let mut covered = BTreeSet::new();
        let mut unlisted = false;
        let privilege = match p {
            ast::Privilege::Read(r, fs) | ast::Privilege::Update(r, fs) => {
                let (rel, t, ctors) = self.target(r)?;
                covering(rel, &ctors, &mut covered);
                unlisted = fs.is_none();
                let fields = self.fields(rel, &ctors, fs.as_ref())?;
                if matches!(p, ast::Privilege::Read(..)) {
                    Granted::Read(t, fields)
                } else {
                    Granted::Update(t, fields)
                }
            }
            ast::Privilege::Insert(r) | ast::Privilege::Delete(r) => {
                let (rel, t, ctors) = self.target(r)?;
                covering(rel, &ctors, &mut covered);
                if matches!(p, ast::Privilege::Insert(_)) {
                    Granted::Insert(t)
                } else {
                    Granted::Delete(t)
                }
            }
            ast::Privilege::Retag(a, b, except) => {
                let (from, x, ctors) = self.target(a)?;
                covering(from, &ctors, &mut covered);
                let (to, y, ctors) = self.target(b)?;
                covering(to, &ctors, &mut covered);
                Granted::Retag(x, y, self.excepted(from, to, except.as_ref())?)
            }
            ast::Privilege::CallFn(f) => match crate::resolve::lookup_value(schema, self.vis, f) {
                Ok(ValueDef::Function(g)) => {
                    let Some(&id) = ids.functions.get(&g) else {
                        return Err(vec![Diag::new(
                            f.span(),
                            format!("`{}` is not a schema function", f.name.text),
                        )]);
                    };
                    self.refs.push((f.name.span, Ref::Function(g)));
                    Granted::CallFn(id)
                }
                Ok(_) => {
                    return Err(vec![Diag::new(
                        f.span(),
                        format!("`{}` is not a function", f.name.text),
                    )]);
                }
                Err(e) => return Err(vec![Diag::new(f.span(), e)]),
            },
            ast::Privilege::Call(..) | ast::Privilege::CallAt(_) => {
                return Err(vec![Diag::new(
                    self.span,
                    "a bundle call grant is deployment state, an ad hoc statement, never an operator in a migration",
                )]);
            }
            ast::Privilege::Capability(c) => {
                return Err(vec![Diag::new(
                    c.span,
                    format!(
                        "the capability grant `{}` is deployment state, an ad hoc statement, never an operator in a migration",
                        c.text
                    ),
                )]);
            }
        };
        Ok(Stated {
            privilege,
            covered,
            unlisted,
        })
    }
}

/// What a deployment revoke matches among rows of `catalog::grants`.
#[derive(Clone, Debug)]
pub enum DeploymentRevoke {
    Capability(String),
    /// A call grant on a bundle or one of its queries, by its target text.
    Call(String),
    /// A family call grant written without a signature, by `FAMILY::q`.
    Family(String),
    /// A family call grant written with a signature, by its tokens.
    FamilyAt(Vec<String>),
    /// A grant on a system relation.
    Relation(Stated),
}

fn lexed(text: &str) -> Vec<String> {
    tokens(text)
        .into_iter()
        .map(|(_, r)| text[r].to_string())
        .collect()
}

/// Returns what privilege `p` of the deployment revoke `text` matches.
///
/// Fail when `p` is an operator privilege.
pub fn deployment_revoke(schema: &Schema, text: &str, p: &ast::Privilege) -> R<DeploymentRevoke> {
    Ok(match p {
        ast::Privilege::Capability(c) => DeploymentRevoke::Capability(c.text.clone()),
        ast::Privilege::Call(ast::CallTarget::Bundle(b), query) => {
            let bundle = format!("{}-{}", b.family, b.version);
            DeploymentRevoke::Call(
                query
                    .as_ref()
                    .map_or(bundle.clone(), |q| format!("{bundle}::{}", q.text)),
            )
        }
        ast::Privilege::Call(ast::CallTarget::Family(f, _), query) => DeploymentRevoke::Family(
            query
                .as_ref()
                .map_or(f.clone(), |q| format!("{f}::{}", q.text)),
        ),
        ast::Privilege::CallAt(s) => DeploymentRevoke::FamilyAt(lexed(&text[s.span.range()])),
        ast::Privilege::CallFn(f) => {
            return Err(vec![Diag::new(
                f.span(),
                "`grant call fn` is a schema operator; write it in a migration",
            )]);
        }
        _ => {
            let vis = crate::resolve::Visible::all(schema);
            let mut r = Resolving {
                schema,
                vis: &vis,
                statement: Statement::Revoke,
                system: true,
                span: Span::new(0, text.len()),
                refs: Vec::new(),
            };
            DeploymentRevoke::Relation(r.privilege(p)?)
        }
    })
}

impl DeploymentRevoke {
    /// Returns the name of the privilege this revoke names.
    pub fn privilege(&self) -> &str {
        match self {
            DeploymentRevoke::Capability(c) => c,
            DeploymentRevoke::Call(_)
            | DeploymentRevoke::Family(_)
            | DeploymentRevoke::FamilyAt(_) => "call",
            DeploymentRevoke::Relation(s) => s.privilege.privilege_name(),
        }
    }

    /// Returns whether this revoke matches a row with privilege `privilege` and target `target`.
    pub fn matches(&self, schema: &Schema, privilege: &str, target: Option<&str>) -> bool {
        if privilege != self.privilege() {
            return false;
        }
        match (self, target) {
            (DeploymentRevoke::Capability(_), None) => true,
            (DeploymentRevoke::Call(t), Some(row)) => t == row,
            (DeploymentRevoke::Family(t), Some(row)) => {
                row == t
                    || row
                        .strip_prefix(t.as_str())
                        .is_some_and(|r| r.starts_with(" at "))
            }
            (DeploymentRevoke::FamilyAt(t), Some(row)) => *t == lexed(row),
            (DeploymentRevoke::Relation(s), Some(row)) => {
                Granted::read_target(schema, privilege, row).is_some_and(|held| s.revokes(&held))
            }
            _ => false,
        }
    }
}

/// Returns whether a row of `privilege` on target `row` is one a row on target `by` covers: the
/// same target text, a query of the bundle `by` names, the family query of another signature, or
/// on a relation the same or fewer constructors and fields.
pub fn target_covers(
    schema: &Schema,
    privilege: &str,
    by: Option<&str>,
    row: Option<&str>,
) -> bool {
    let (Some(by), Some(row)) = (by, row) else {
        return by == row;
    };
    if by == row {
        return true;
    }
    let family = |t: &'_ str| t.split_once(" at ").map(|(q, _)| q.to_owned());
    if privilege == "call"
        && (row.strip_prefix(by).is_some_and(|q| q.starts_with("::"))
            || family(by).is_some_and(|q| family(row).is_some_and(|r| q == r)))
    {
        return true;
    }
    let (Some(by), Some(row)) = (
        Granted::read_target(schema, privilege, by),
        Granted::read_target(schema, privilege, row),
    ) else {
        return false;
    };
    let within = |a: &GrantTarget, b: &GrantTarget| {
        a.relation == b.relation && b.ctors(schema).is_subset(&a.ctors(schema))
    };
    let fewer = |a: &GrantFields, b: &GrantFields| b.ids.iter().all(|x| a.ids.contains(x));
    match (&by, &row) {
        (Granted::Read(a, x), Granted::Read(b, y))
        | (Granted::Update(a, x), Granted::Update(b, y)) => within(a, b) && fewer(x, y),
        (Granted::Insert(a), Granted::Insert(b)) | (Granted::Delete(a), Granted::Delete(b)) => {
            within(a, b)
        }
        (Granted::Retag(a, b, _), Granted::Retag(x, y, _)) => within(a, x) && within(b, y),
        _ => by == row,
    }
}

/// The role of the schema owner, the grantor of every operator grant.
pub const OWNER_ROLE: &str = "owner";

/// The fields of a row of `catalog::grants` that decide which rows its grant option supports.
#[derive(Clone, Copy, Debug)]
pub struct GrantRow<'a> {
    pub grantor: &'a str,
    pub grantee: &'a str,
    pub privilege: &'a str,
    pub target: Option<&'a str>,
    pub grant_option: bool,
    pub dormant: bool,
}

impl GrantRow<'_> {
    fn covers(&self, schema: &Schema, row: &GrantRow<'_>) -> bool {
        self.grant_option
            && self.grantee == row.grantor
            && self.privilege == row.privilege
            && target_covers(schema, row.privilege, self.target, row.target)
    }

    /// Returns whether deleting this row's option takes the support of `row`: its grantee, not the
    /// schema owner, gave `row` under the option.
    pub fn supports(&self, schema: &Schema, row: &GrantRow<'_>) -> bool {
        self.grantee != OWNER_ROLE && self.covers(schema, row)
    }

    /// Returns whether this row still supports `row` when another option of its grantee is lost:
    /// it covers `row`, is not dormant, and its grantor is another role.
    pub fn keeps(&self, schema: &Schema, row: &GrantRow<'_>) -> bool {
        !self.dormant && self.grantor != self.grantee && self.covers(schema, row)
    }
}

/// Returns whether a grant with grant option of `new` would close a cycle among the options of
/// `rows`, dormant ones included: its grantee is its grantor, or a role its grantor's option
/// descends from.
pub fn closes_option_cycle(schema: &Schema, rows: &[GrantRow<'_>], new: GrantRow<'_>) -> bool {
    let overlaps = |e: &GrantRow<'_>| {
        e.grant_option
            && e.privilege == new.privilege
            && (target_covers(schema, new.privilege, new.target, e.target)
                || target_covers(schema, new.privilege, e.target, new.target))
    };
    let mut seen = BTreeSet::new();
    let mut frontier = vec![new.grantee];
    while let Some(role) = frontier.pop() {
        if role == new.grantor {
            return true;
        }
        if seen.insert(role) {
            frontier.extend(
                rows.iter()
                    .filter(|e| e.grantor == role && overlaps(e))
                    .map(|e| e.grantee),
            );
        }
    }
    false
}
