use std::collections::BTreeSet;
use std::sync::Arc;

use adtb_lang::capability::RootKind;
use adtb_lang::kernel::{Terms, Tm, Var};
use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::{Committed, Schema, TypeDefKind};
use adtb_lang::types::{DenialIdx, RelId, Ty, TypeId};

use crate::exec::types::ctors;
use crate::exec::{self, Held, Val};
use crate::plan::{self, Catalog};
use crate::runtime::Runtime;

use super::change::ChangeSet;
use super::delta::{self, Delta, Input, Part};
use super::eval::{Arena, Env, Stmt};
use super::schema::RefField;
use super::violation::{Violations, Violator, violate};
use super::{Broken, Error};

/// A constraint or check of the schema, as its incremental checks ready to run.
pub struct Denial {
    of: DenialIdx,
    pub name: String,
    /// The relation variables the query reads.
    pub reads: BTreeSet<RelId>,
    pub deltas: Vec<(Delta, Arc<exec::Program>)>,
}

impl Denial {
    /// Returns the constraint or check this is the denial of.
    pub fn of(&self) -> DenialIdx {
        self.of
    }
}

/// Returns the incremental checks of every constraint and check of `schema`, their terms added
/// to `terms`, a copy of `normal`'s arena.
///
/// Errors:
/// - [`Error::Refused`] when a constraint has no index-bounded incremental check.
pub fn denials(
    schema: &Committed,
    normal: &NormalProgram,
    catalog: &Catalog,
    terms: &mut Terms,
) -> Result<Vec<Denial>, Error> {
    let checks = delta::compile(schema, normal, terms, catalog).map_err(|e| match e {
        delta::Unadmitted::Refused(why) => {
            Error::Refused(why.into_iter().map(|r| (r.constraint, r.detail)).collect())
        }
        delta::Unadmitted::Internal(b) => b.into(),
    })?;
    let mut out = Vec::new();
    for c in checks {
        let of = c.of();
        let mut deltas = Vec::new();
        for d in c.deltas {
            let program = Arc::new(exec::compile(schema, catalog, &d.plan, None)?);
            deltas.push((d, program));
        }
        out.push(Denial {
            of,
            name: c.name,
            reads: c.reads,
            deltas,
        });
    }
    Ok(out)
}

/// Returns each constraint and check of `schema` without an index-bounded incremental check
/// over the indexes `catalog` describes, or with an incremental check or a query that validates
/// it over the empty change that the executor cannot compile, by qualified name, with why; empty
/// when the schema admits every one.
///
/// Errors:
/// - The guarantee of the checker that compiling a check breaks.
pub fn refusals(
    schema: &Committed,
    normal: &NormalProgram,
    catalog: &Catalog,
) -> Result<Vec<(String, String)>, Broken> {
    let mut terms = normal.terms().clone();
    let checks = match delta::compile(schema, normal, &mut terms, catalog) {
        Ok(checks) => checks,
        Err(delta::Unadmitted::Refused(why)) => {
            return Ok(why.into_iter().map(|r| (r.constraint, r.detail)).collect());
        }
        Err(delta::Unadmitted::Internal(b)) => return Err(b),
    };
    let mut out = Vec::new();
    for c in &checks {
        for d in &c.deltas {
            let why = uncompiled(exec::compile(schema, catalog, &d.plan, None))?;
            out.extend(why.map(|why| (c.name.clone(), why)));
        }
    }
    let roots: Vec<(Tm, Vec<Var>)> = checks.iter().map(|c| (c.whole(), Vec::new())).collect();
    let terms =
        adtb_lang::kernel::ValidTerms::new(schema, terms, &roots).map_err(Broken::IllTyped)?;
    for c in &checks {
        let options = plan::Options::default();
        let whole = plan::plan(
            schema,
            normal,
            catalog,
            terms.valid(),
            c.whole(),
            RootKind::Constraint,
            options,
        )
        .map_err(Broken::WholePlan)?;
        let why = uncompiled(exec::compile(schema, catalog, &whole, None))?;
        out.extend(why.map(|why| (c.name.clone(), why)));
    }
    out.dedup();
    Ok(out)
}

#[cfg(any(test, feature = "testing"))]
static IDLE_CHECKED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Returns how many incremental checks whose triggering change was empty this process has run.
#[cfg(any(test, feature = "testing"))]
pub fn idle_deltas_checked() -> u64 {
    IDLE_CHECKED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Returns why the executor cannot compile a program, or `None` when it compiled.
fn uncompiled<T>(compiled: Result<T, exec::Error>) -> Result<Option<String>, Broken> {
    match compiled {
        Ok(_) => Ok(None),
        Err(exec::Error::Unsupported(why)) => Ok(Some(why)),
        Err(e) => Err(Broken::Compile(e)),
    }
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
            .all_by_key(f.target, k)?
            .iter()
            .any(|(c, _, _)| f.ctors.as_ref().is_none_or(|s| s.contains(c))))
    }

    /// Check, after the change set is applied, every constraint it can violate: references,
    /// newtype domains, and the schema's constraints and checks.
    pub fn check(&self, set: &ChangeSet, viol: &mut Violations) -> Result<(), Error> {
        self.check_rows(set, viol)?;
        let mut moved: Held<BTreeSet<(RelId, Val)>> = self.rows_in().held();
        for ((r, c), v) in set.minus() {
            let k = self.key_of(*r, v)?;
            if !self.all_by_key(*r, &k)?.iter().any(|(d, _, _)| d == c) {
                moved.insert((*r, k))?;
            }
        }
        let targets: BTreeSet<RelId> = moved.iter().map(|(r, _)| *r).collect();
        for (r, fields) in self.referrers(&targets) {
            for (_, (c, u)) in self.referring(r, &fields, &moved)? {
                for f in &fields[c as usize] {
                    if let Some(k) = Self::names(f, &u)?
                        && moved.contains(&(f.target, k.clone()))
                        && !self.reaches(f, &k)?
                    {
                        violate(
                            viol,
                            reference(r, c, f),
                            &self.field_name(r, c, f),
                            [u.clone()],
                        )?;
                    }
                }
            }
        }
        let touched: BTreeSet<RelId> = (set.plus().iter().chain(set.minus()))
            .map(|((r, _), _)| *r)
            .collect();
        for d in &self.db.denials {
            if d.reads.is_disjoint(&touched) {
                continue;
            }
            for (delta, program) in &d.deltas {
                let params: Vec<(Var, Held<Val>)> = delta
                    .inputs
                    .iter()
                    .map(|i| Ok((i.var, self.input_rows(i, set)?)))
                    .collect::<Result<_, Error>>()?;
                let idle = delta.trigger.is_some_and(|t| {
                    params
                        .iter()
                        .any(|(v, rows)| *v == t && matches!(&**rows, Val::Set(s) if s.is_empty()))
                });
                if idle && !cfg!(any(test, feature = "testing")) {
                    continue;
                }
                let call = self
                    .db
                    .call(&self.store, self.options, &self.stop, self.grant())?;
                let bound = program.clone().bind(|v| {
                    params
                        .iter()
                        .find(|(p, _)| *p == v)
                        .map(|(_, rows)| Val::clone(rows))
                })?;
                match exec::execute(self.db.rt.clone(), bound, call)?.into_inner() {
                    Val::Set(report) if idle => {
                        #[cfg(any(test, feature = "testing"))]
                        IDLE_CHECKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if !report.is_empty() {
                            return Err(Broken::IdleDeltaReports(d.name.clone()).into());
                        }
                    }
                    Val::Set(report) => {
                        violate(
                            viol,
                            Violator::Denial(d.of()),
                            &d.name,
                            report.iter().cloned(),
                        )?;
                    }
                    other => {
                        return Err(Broken::DenialNotSet(other).into());
                    }
                }
            }
        }
        Ok(())
    }

    /// Returns the rows of change set `set` that `input` names, held in the statement's rows'
    /// grant.
    fn input_rows(&self, input: &Input, set: &ChangeSet) -> Result<Held<Val>, Error> {
        let mut out: Held<BTreeSet<Val>> = self.rows_in().held();
        for c in &input.ctors {
            if input.part != Part::Deleted {
                for v in ChangeSet::of(set.plus(), input.rel, *c) {
                    out.insert(v.clone())?;
                }
            }
            if input.part != Part::Inserted {
                for v in ChangeSet::of(set.minus(), input.rel, *c) {
                    out.insert(v.clone())?;
                }
            }
        }
        Ok(out.into_value()?)
    }

    /// Check the references and newtype domains of every row `set` inserts.
    pub fn check_rows(&self, set: &ChangeSet, viol: &mut Violations) -> Result<(), Error> {
        for ((r, c), n) in set.plus() {
            for f in &self.rel(*r).ctors[*c as usize].refs {
                if let Some(k) = Self::names(f, n)?
                    && !self.reaches(f, &k)?
                {
                    let name = self.field_name(*r, *c, f);
                    violate(viol, reference(*r, *c, f), &name, [n.clone()])?;
                }
            }
        }
        for ((r, _), v) in set.plus() {
            let elem = self.rel(*r).elem.clone();
            self.check_domains(v, v, &elem, viol)?;
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
                    if let Some((var, body)) = self.db.normal.domains().get(def) {
                        let env: Env = [(*var, v.clone())].into();
                        if !self
                            .eval(Arena::Program(RootKind::Domain), *body, &env)?
                            .truth()?
                        {
                            let name = domain_name(schema, *def);
                            violate(viol, Violator::Domain(*def), &name, [row.clone()])?;
                        }
                    }
                    self.check_domains(row, v, &base.subst(args), viol)
                }
                TypeDefKind::Sum { .. } => {
                    let c = v.ctor_of()?;
                    let fields = ctors(schema, ty).expect("a sum")[c as usize].clone();
                    for ((_, t), x) in fields.iter().zip(v.fields()?) {
                        self.check_domains(row, x, t, viol)?;
                    }
                    Ok(())
                }
            },
            Ty::Record(fs) => {
                for ((_, t), x) in fs.iter().zip(v.fields()?) {
                    self.check_domains(row, x, t, viol)?;
                }
                Ok(())
            }
            Ty::Sum { ctors: cs, .. } => {
                for ((_, t), x) in cs[v.ctor_of()? as usize].1.iter().zip(v.fields()?) {
                    self.check_domains(row, x, t, viol)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Returns what a violation of reference field `f` of constructor `c` of `r` violates.
fn reference(r: RelId, c: u32, f: &RefField) -> Violator {
    Violator::Reference(r, (!f.header).then_some(c), f.at)
}

fn domain_name(schema: &Schema, t: TypeId) -> String {
    let d = schema.ty(t);
    format!("{}::{}.domain", schema.module(d.module).name, d.name)
}
