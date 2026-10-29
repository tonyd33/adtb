use super::*;

const USER_IDS: &str = "every object of a user module has a catalog id";

/// A privilege a log `grant` gives a role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogGrant {
    pub role: String,
    pub privilege: Granted,
    pub grant_option: bool,
}

/// A privilege on schema objects, each named by its catalog id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Granted {
    Read(GrantTarget, GrantFields),
    Insert(GrantTarget),
    Update(GrantTarget, GrantFields),
    Delete(GrantTarget),
    Retag(GrantTarget, GrantTarget),
    CallFn(u64),
}

/// A relation variable, or the constructors of it a grant selects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantTarget {
    pub relation: u64,
    pub sel: GrantSel,
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
            Granted::Retag(a, b) => Granted::Retag(a.mapped(f)?, b.mapped(f)?),
            Granted::CallFn(x) => Granted::CallFn(f(IdKind::Function, *x)?),
        })
    }

    /// Returns whether a revoke of `self` takes away `held`: the same privilege on the same
    /// target, with the same field list, or with none when `self` states none.
    fn revokes(&self, held: &Granted) -> bool {
        let fields = |a: &GrantFields, b: &GrantFields| {
            a.listed == b.listed && (!a.listed || a.ids == b.ids)
        };
        match (self, held) {
            (Granted::Read(a, x), Granted::Read(b, y))
            | (Granted::Update(a, x), Granted::Update(b, y)) => a == b && fields(x, y),
            _ => self == held,
        }
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

/// What a role holds of a privilege: its place among the grants and its grant option, `None`
/// when it holds nothing.
#[derive(Clone, Debug)]
pub(super) struct HeldGrant {
    pub(super) role: String,
    pub(super) privilege: Granted,
    pub(super) held: Option<(usize, bool)>,
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
                (Some(i), Some((_, grant_option))) => self.grants[i].grant_option = grant_option,
                (Some(i), None) => {
                    self.grants.remove(i);
                }
                (None, Some((i, grant_option))) => self.grants.insert(
                    i.min(self.grants.len()),
                    LogGrant {
                        role: h.role,
                        privilege,
                        grant_option,
                    },
                ),
                (None, None) => {}
            }
        }
    }
}

impl Cx<'_> {
    /// Add each privilege to the role's log grants, keeping one entry per privilege, and settle
    /// each grant of the role that a structural operator of this migration could not carry to a
    /// constructor the privilege covers.
    pub(super) fn grant(&mut self, g: &ast::Grant) -> R<()> {
        for (privilege, covered) in self.granted(&g.privileges)? {
            self.src
                .unstated
                .retain(|u| !u.stated_by(&g.to.text, &privilege, &covered));
            match self
                .src
                .grants
                .iter_mut()
                .find(|x| x.role == g.to.text && x.privilege == privilege)
            {
                Some(held) => held.grant_option |= g.grant_option,
                None => self.src.grants.push(LogGrant {
                    role: g.to.text.clone(),
                    privilege,
                    grant_option: g.grant_option,
                }),
            }
        }
        Ok(())
    }

    pub(super) fn revoke(&mut self, r: &ast::Revoke) -> R<()> {
        let gone: Vec<Granted> = self
            .granted(&r.privileges)?
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        self.src
            .grants
            .retain(|x| x.role != r.from.text || !gone.iter().any(|p| p.revokes(&x.privilege)));
        Ok(())
    }

    /// Resolve the privileges of a log grant or revoke to the catalog ids of what they name, each
    /// with the ids of the constructors its targets cover. A field list it omits is the fields
    /// its constructors have now.
    fn granted(&self, privileges: &[ast::Privilege]) -> R<Vec<(Granted, BTreeSet<u64>)>> {
        let schema = self.schema()?;
        let vis = crate::resolve::Visible::module(
            schema.module_named(&self.module).unwrap_or(schema::PRELUDE),
        );
        let refs = RefCell::new(Vec::new());
        let covered = RefCell::new(BTreeSet::new());
        let target = |r: &ast::RelationRef,
                      fields: Option<&[ast::Ident]>|
         -> R<(GrantTarget, GrantFields)> {
            let rel = match crate::resolve::lookup_relation(&schema, &vis, &r.relation) {
                Ok(rel) => rel,
                Err(e) => return self.err_at(r.span, e),
            };
            let rd = schema.relation(rel);
            if schema.module(rd.module).engine {
                return self.err_at(
                    r.span,
                    "a grant on a `catalog::` or `sys::` relation is deployment state, an ad hoc statement, never an operator in a migration",
                );
            }
            let def = rd.elem_def;
            let ids = &schema.ids;
            let ctor_id = |c: u32| *ids.ctors.get(&(def, c)).expect(USER_IDS);
            let mut refs = refs.borrow_mut();
            refs.push((r.relation.name.span, Ref::Relation(rel)));
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
                        refs.push(match schema.ty(def).ctor_index(&x.text) {
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
                            let mut ids: Vec<u64> = ctors.iter().map(|&c| ctor_id(c)).collect();
                            ids.sort_unstable();
                            (GrantSel::Ctors(ids), ctors)
                        }
                    }
                }
            };
            covered
                .borrow_mut()
                .extend(ctors.iter().map(|&c| ctor_id(c)));
            let fields = match fields {
                None => {
                    let mut out: Vec<u64> = ids
                        .fields
                        .iter()
                        .filter(|((d, c, _), _)| *d == def && c.is_none_or(|c| ctors.contains(&c)))
                        .map(|(_, &id)| id)
                        .collect();
                    out.sort_unstable();
                    GrantFields {
                        ids: out,
                        listed: false,
                    }
                }
                Some(fs) => {
                    let mut out = Vec::new();
                    for f in fs {
                        let found: Vec<u64> = std::iter::once(None)
                            .chain(ctors.iter().map(|&c| Some(c)))
                            .filter_map(|c| ids.fields.get(&(def, c, f.text.clone())).copied())
                            .collect();
                        if found.is_empty() {
                            return self.err_at(
                                f.span,
                                format!("`{}` has no field `{}`", rd.name, f.text),
                            );
                        }
                        out.extend(found);
                    }
                    out.sort_unstable();
                    out.dedup();
                    GrantFields {
                        ids: out,
                        listed: true,
                    }
                }
            };
            let relation = *ids.relations.get(&rel).expect(USER_IDS);
            Ok((GrantTarget { relation, sel }, fields))
        };
        let mut out = Vec::new();
        for p in privileges {
            covered.borrow_mut().clear();
            let privilege = match p {
                ast::Privilege::Read(r, fs) => {
                    let (t, fs) = target(r, fs.as_deref())?;
                    Granted::Read(t, fs)
                }
                ast::Privilege::Update(r, fs) => {
                    let (t, fs) = target(r, fs.as_deref())?;
                    Granted::Update(t, fs)
                }
                ast::Privilege::Insert(r) => Granted::Insert(target(r, None)?.0),
                ast::Privilege::Delete(r) => Granted::Delete(target(r, None)?.0),
                ast::Privilege::Retag(a, b) => {
                    Granted::Retag(target(a, None)?.0, target(b, None)?.0)
                }
                ast::Privilege::CallFn(f) => match crate::resolve::lookup_value(&schema, &vis, f) {
                    Ok(ValueDef::Function(g)) => {
                        let Some(&id) = schema.ids.functions.get(&g) else {
                            return self.err_at(
                                f.span(),
                                format!("`{}` is not a schema function", f.name.text),
                            );
                        };
                        refs.borrow_mut().push((f.name.span, Ref::Function(g)));
                        Granted::CallFn(id)
                    }
                    Ok(_) => {
                        return self
                            .err_at(f.span(), format!("`{}` is not a function", f.name.text));
                    }
                    Err(e) => return self.err_at(f.span(), e),
                },
                ast::Privilege::Call(..) => {
                    return self.err(
                        "a bundle call grant is deployment state, an ad hoc statement, never an operator in a migration",
                    );
                }
                ast::Privilege::Capability(c) => {
                    return self.err_at(
                        c.span,
                        format!(
                            "the capability grant `{}` is deployment state, an ad hoc statement, never an operator in a migration",
                            c.text
                        ),
                    );
                }
            };
            out.push((privilege, covered.borrow().clone()));
        }
        self.note(&schema, &refs.into_inner());
        Ok(out)
    }
}
