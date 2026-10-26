use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::kernel::{Kind, Terms, Tm, Transition, Var};
use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::{Schema, TypeDefKind};
use adtb_lang::types::{CtorSet, RelId, Ty, TypeId};

use crate::exec::Val;
use crate::exec::types::ctors;
use crate::runtime::Runtime;

use super::eval::{Arena, Env, Stmt, Violations, pairs, violate};
use super::schema::RefField;
use super::{Error, Rows};

/// A constraint or check of the schema, as its denial query in the program arena.
#[derive(Debug)]
pub struct Denial {
    pub name: String,
    pub term: Tm,
    /// The variable standing for each transition the query reads.
    pub transitions: Vec<(Var, Transition, RelId, Option<CtorSet>)>,
    /// The relation variables the query reads.
    pub reads: BTreeSet<RelId>,
}

/// Replace each transition read under `t` in place by a variable, and return the variables
/// with the relations every read under `t` names.
fn scan(
    terms: &mut Terms,
    t: Tm,
    seen: &mut BTreeSet<Tm>,
    out: &mut Vec<(Var, Transition, RelId, Option<CtorSet>)>,
    reads: &mut BTreeSet<RelId>,
) {
    if !seen.insert(t) {
        return;
    }
    match terms.kind(t).clone() {
        Kind::Transition(x, r, s) => {
            let ty = terms.ty(t).clone();
            let v = terms.var(&format!("%transition{}", t.0), ty);
            terms.nodes[t.0 as usize].kind = Kind::Var(v);
            out.push((v, x, r, s));
            reads.insert(r);
        }
        Kind::Rel(r, _) => {
            reads.insert(r);
        }
        Kind::Deref { key, .. } => {
            if let Ty::Key { rel, .. } = terms.ty(key) {
                reads.insert(*rel);
            }
            scan(terms, key, seen, out, reads);
        }
        _ => {
            for c in terms.children(t) {
                scan(terms, c, seen, out, reads);
            }
        }
    }
}

/// Returns the denial query of every constraint and check, with its transition reads replaced
/// by variables in `terms`, a copy of `normal`'s arena.
pub fn denials(schema: &Schema, normal: &NormalProgram, terms: &mut Terms) -> Vec<Denial> {
    let named = schema
        .constraints
        .iter()
        .map(|c| (c.module, &c.name))
        .zip(&normal.constraints)
        .chain(
            schema
                .checks
                .iter()
                .map(|c| (c.module, &c.name))
                .zip(&normal.checks),
        );
    named
        .map(|((module, name), t)| {
            let mut transitions = Vec::new();
            let mut reads = BTreeSet::new();
            scan(
                terms,
                *t,
                &mut BTreeSet::new(),
                &mut transitions,
                &mut reads,
            );
            Denial {
                name: format!("{}::{name}", schema.module(module).name),
                term: *t,
                transitions,
                reads,
            }
        })
        .collect()
}

impl<R: Runtime> Stmt<'_, R> {
    fn field_name(&self, rel: RelId, c: u32, f: &RefField) -> String {
        let r = self.rel(rel);
        if f.header {
            format!("{}.{}", r.qualified, f.name)
        } else {
            format!("{}.{}.{}", r.qualified, r.ctors[c as usize].name, f.name)
        }
    }

    /// Returns whether the statement's reads see a row of `f`'s target, in its constructor set,
    /// with key `k`.
    fn reaches(&self, f: &RefField, k: &Val) -> Result<bool, Error> {
        Ok(self
            .by_key(f.target, k)?
            .is_some_and(|(c, _, _)| f.ctors.as_ref().is_none_or(|s| s.contains(&c))))
    }

    /// Check, after the change set is applied, every constraint it can violate: references,
    /// uniques no index holds, newtype domains, and the schema's constraints and checks.
    pub fn check(&self, plus: &Rows, minus: &Rows, viol: &mut Violations) -> Result<(), Error> {
        for ((r, c), rows) in plus {
            for f in &self.rel(*r).ctors[*c as usize].refs {
                for n in rows {
                    if let Some(k) = Self::names(f, n)
                        && !self.reaches(f, &k)?
                    {
                        violate(viol, &self.field_name(*r, *c, f), [n.clone()]);
                    }
                }
            }
        }
        let mut moved: BTreeMap<RelId, BTreeSet<Val>> = BTreeMap::new();
        for ((r, c), rows) in minus {
            for v in rows {
                let k = self.key_of(*r, v);
                let now = self.by_key(*r, &k)?;
                if now.is_none_or(|(d, _, _)| d != *c) {
                    moved.entry(*r).or_default().insert(k);
                }
            }
        }
        let targets: BTreeSet<RelId> = moved.keys().copied().collect();
        for (r, fields) in self.referrers(&targets) {
            for (c, u) in self.rows(r, None)? {
                for f in &fields[c as usize] {
                    if let Some(k) = Self::names(f, &u)
                        && moved.get(&f.target).is_some_and(|ks| ks.contains(&k))
                        && !self.reaches(f, &k)?
                    {
                        violate(viol, &self.field_name(r, c, f), [u.clone()]);
                    }
                }
            }
        }
        for rel in plus.keys().map(|(r, _)| *r).collect::<BTreeSet<_>>() {
            self.check_uniques(rel, plus, viol)?;
        }
        for ((r, _), rows) in plus {
            for v in rows {
                let elem = self.rel(*r).elem.clone();
                self.check_domains(v, v, &elem, viol)?;
            }
        }
        let touched: BTreeSet<RelId> = plus.keys().chain(minus.keys()).map(|(r, _)| *r).collect();
        for d in &self.db.denials {
            if d.reads.is_disjoint(&touched) {
                continue;
            }
            let mut env = Env::new();
            for (var, x, r, s) in &d.transitions {
                env.insert(
                    *var,
                    Val::set(self.transition(*x, *r, s.as_deref(), plus, minus)?),
                );
            }
            match self.eval(Arena::Program, d.term, &env)? {
                Val::Set(report) => violate(viol, &d.name, report.iter().cloned()),
                other => {
                    return Err(Error::Internal(format!(
                        "a denial query returns a set, found {other:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn transition(
        &self,
        x: Transition,
        rel: RelId,
        of: Option<&[u32]>,
        plus: &Rows,
        minus: &Rows,
    ) -> Result<BTreeSet<Val>, Error> {
        let pick = |rows: &Rows| -> BTreeSet<Val> {
            rows.iter()
                .filter(|((r, c), _)| *r == rel && of.is_none_or(|s| s.contains(c)))
                .flat_map(|(_, vs)| vs.iter().cloned())
                .collect()
        };
        Ok(match x {
            Transition::Inserted => pick(plus),
            Transition::Deleted => pick(minus),
            Transition::Prior => {
                let inserted = pick(plus);
                let mut out: BTreeSet<Val> = self
                    .rows(rel, of)?
                    .into_iter()
                    .map(|(_, v)| v)
                    .filter(|v| !inserted.contains(v))
                    .collect();
                out.extend(pick(minus));
                out
            }
        })
    }

    /// Check the uniques of `rel` that no index holds against the rows the change set inserts.
    fn check_uniques(&self, rel: RelId, plus: &Rows, viol: &mut Violations) -> Result<(), Error> {
        let r = self.rel(rel);
        for (i, u) in r.uniques.iter().enumerate() {
            if u.index.is_some() {
                continue;
            }
            let mut wanted = BTreeSet::new();
            for ((x, c), rows) in plus {
                if *x != rel || !u.scope.contains(c) {
                    continue;
                }
                for v in rows {
                    if let Some(vs) = self.unique_values(rel, i, v)? {
                        wanted.insert(vs);
                    }
                }
            }
            if wanted.is_empty() {
                continue;
            }
            let mut groups: BTreeMap<Vec<Val>, BTreeMap<Val, Val>> = BTreeMap::new();
            for (_, row) in self.rows(rel, Some(&u.scope))? {
                if let Some(vs) = self.unique_values(rel, i, &row)?
                    && wanted.contains(&vs)
                {
                    groups
                        .entry(vs)
                        .or_default()
                        .insert(self.key_of(rel, &row), row);
                }
            }
            violate(viol, &u.name, pairs(groups.values()));
        }
        Ok(())
    }

    /// Check every value of a newtype with a domain inside `v`, of type `ty`, a part of `row`.
    fn check_domains(
        &self,
        row: &Val,
        v: &Val,
        ty: &Ty,
        viol: &mut Violations,
    ) -> Result<(), Error> {
        let schema = &self.db.schema;
        match ty {
            Ty::Data { def, args, .. } => match &schema.ty(*def).kind {
                TypeDefKind::Newtype { base, .. } => {
                    if let Some((var, body)) = self.db.normal.domains.get(def) {
                        let env: Env = [(*var, v.clone())].into();
                        if !self.eval(Arena::Program, *body, &env)?.truth() {
                            violate(viol, &domain_name(schema, *def), [row.clone()]);
                        }
                    }
                    self.check_domains(row, v, &base.subst(args), viol)
                }
                TypeDefKind::Sum { .. } => {
                    let c = v.ctor_of();
                    let fields = ctors(schema, ty).expect("a sum")[c as usize].clone();
                    for ((_, t), x) in fields.iter().zip(v.fields()) {
                        self.check_domains(row, x, t, viol)?;
                    }
                    Ok(())
                }
            },
            Ty::Record(fs) => {
                for ((_, t), x) in fs.iter().zip(v.fields()) {
                    self.check_domains(row, x, t, viol)?;
                }
                Ok(())
            }
            Ty::Sum { ctors: cs, .. } => {
                for ((_, t), x) in cs[v.ctor_of() as usize].1.iter().zip(v.fields()) {
                    self.check_domains(row, x, t, viol)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn domain_name(schema: &Schema, t: TypeId) -> String {
    let d = schema.ty(t);
    format!("{}::{}.domain", schema.module(d.module).name, d.name)
}
