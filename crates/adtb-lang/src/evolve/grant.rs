use super::*;

const USER_IDS: &str = "every object of a user module has a catalog id";

/// A privilege a log `grant` gives a role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogGrant {
    pub role: String,
    pub privilege: Granted,
    pub grant_option: bool,
}

/// A privilege on schema objects, each named by its catalog id. A field list is `None` when the
/// grant states none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Granted {
    Read(GrantTarget, Option<Vec<u64>>),
    Insert(GrantTarget),
    Update(GrantTarget, Option<Vec<u64>>),
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantSel {
    Whole,
    Ctors(Vec<u64>),
    Set(u64),
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
    /// it lacks back at its place.
    pub(super) fn hold(&mut self, back: Vec<HeldGrant>) {
        for h in back {
            let at = self
                .grants
                .iter()
                .position(|x| x.role == h.role && x.privilege == h.privilege);
            match (at, h.held) {
                (Some(i), Some((_, grant_option))) => self.grants[i].grant_option = grant_option,
                (Some(i), None) => {
                    self.grants.remove(i);
                }
                (None, Some((i, grant_option))) => self.grants.insert(
                    i.min(self.grants.len()),
                    LogGrant {
                        role: h.role,
                        privilege: h.privilege,
                        grant_option,
                    },
                ),
                (None, None) => {}
            }
        }
    }
}

impl Cx<'_> {
    /// Add each privilege to the role's log grants, keeping one entry per privilege.
    pub(super) fn grant(&mut self, g: &ast::Grant) -> R<()> {
        for privilege in self.granted(&g.privileges)? {
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
        let gone = self.granted(&r.privileges)?;
        self.src
            .grants
            .retain(|x| x.role != r.from.text || !gone.contains(&x.privilege));
        Ok(())
    }

    /// Resolve the privileges of a log grant or revoke to the catalog ids of what they name.
    fn granted(&self, privileges: &[ast::Privilege]) -> R<Vec<Granted>> {
        let schema = self.schema()?;
        let vis = crate::resolve::Visible::module(
            schema.module_named(&self.module).unwrap_or(schema::PRELUDE),
        );
        let refs = RefCell::new(Vec::new());
        let target = |r: &ast::RelationRef,
                      fields: Option<&[ast::Ident]>|
         -> R<(GrantTarget, Option<Vec<u64>>)> {
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
                        _ => (
                            GrantSel::Ctors(ctors.iter().map(|&c| ctor_id(c)).collect()),
                            ctors,
                        ),
                    }
                }
            };
            let fields = match fields {
                None => None,
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
                    Some(out)
                }
            };
            let relation = *ids.relations.get(&rel).expect(USER_IDS);
            Ok((GrantTarget { relation, sel }, fields))
        };
        let mut out = Vec::new();
        for p in privileges {
            out.push(match p {
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
                ast::Privilege::Retag(a, b) => Granted::Retag(target(a, None)?.0, target(b, None)?.0),
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
            });
        }
        self.note(&schema, &refs.into_inner());
        Ok(out)
    }
}
