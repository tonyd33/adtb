use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use adtb_lang::ast::FnBody;
use adtb_lang::kernel::{Const, Kind, Terms, Tm};
use adtb_lang::schema::Schema;
use adtb_lang::types::{Base, FnId, Ty};

use crate::catalog::layout::batch_type;

use super::frame::{Frame, Sel};
use super::host::{HostCall, Hosts};
use super::prim::{self, Op};
use super::store::conv;
use super::types::{At, base, ctors, field, key_fields, repr};
use super::value::{F32, F64, Val};
use super::{Error, Fault};

/// An operand of an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opd {
    Var(u32),
    Reg(u32),
    Lit(u32),
}

#[derive(Clone, Debug)]
pub enum Ins {
    Prim {
        op: Op,
        args: Vec<Opd>,
        dst: u32,
    },
    Record {
        fields: Vec<Opd>,
        dst: u32,
    },
    /// Build constructor `ctor` from `fields`, already in the order a `Val::Ctor` holds them.
    Ctor {
        ctor: u32,
        fields: Vec<Opd>,
        dst: u32,
    },
    Field {
        src: Opd,
        at: At,
        dst: u32,
    },
    /// The key of a row: one field's value, or the record of the fields at `at`, in name order.
    Key {
        src: Opd,
        at: Vec<At>,
        single: bool,
        dst: u32,
    },
    Tag {
        src: Opd,
        dst: u32,
    },
    Singleton {
        src: Opd,
        dst: u32,
    },
    Union {
        a: Opd,
        b: Opd,
        dst: u32,
    },
    /// The elements of a set whose constructor `keep` holds.
    Restrict {
        src: Opd,
        keep: Vec<bool>,
        dst: u32,
    },
    Bind {
        var: u32,
        src: Opd,
    },
    If {
        cond: Opd,
        then: Block,
        els: Block,
        dst: u32,
    },
    /// One arm per constructor position, `None` for those the scrutinee cannot hold.
    Case {
        scrut: Opd,
        arms: Vec<Option<(u32, Block)>>,
        dst: u32,
    },
    Try {
        body: Block,
        dst: u32,
    },
    Caller {
        dst: u32,
    },
    /// Calls a host function once for every selected row.
    Host {
        call: Box<HostCall>,
        args: Vec<Opd>,
        dst: u32,
    },
    /// Rejects the statement's input with the text of `msg` at the first selected row.
    Reject {
        msg: Opd,
    },
    Unreachable,
}

/// Instructions, and the operand holding their value once they have run.
#[derive(Clone, Debug)]
pub struct Block {
    pub ins: Vec<Ins>,
    pub out: Opd,
}

/// A compiled scalar expression.
#[derive(Clone, Debug)]
pub struct Code {
    block: Block,
    regs: u32,
    lits: Vec<Val>,
}

/// The values the instructions of a frame's code write, one column per register.
struct Regs(Vec<Vec<Val>>);

/// Where faults of rows go while they run.
#[derive(Debug, Default)]
pub struct Catch {
    /// Whether a fault is caught; when false it aborts the evaluation.
    pub on: bool,
    pub faults: Vec<(u32, Fault)>,
}

impl Catch {
    pub fn on() -> Catch {
        Catch {
            on: true,
            faults: Vec::new(),
        }
    }

    pub fn off() -> Catch {
        Catch::default()
    }

    /// Record `f` at `row`, or return it as the evaluation's error.
    pub fn fault(&mut self, row: u32, f: Fault) -> Result<(), Error> {
        if self.on {
            self.faults.push((row, f));
            Ok(())
        } else {
            Err(Error::Fault(f))
        }
    }

    /// Remove from `sel` the rows faulted since the catch held `since` faults.
    pub fn drop_faulted(&self, since: usize, sel: &mut Sel) {
        if self.faults.len() == since {
            return;
        }
        let bad: BTreeSet<u32> = self.faults[since..].iter().map(|(r, _)| *r).collect();
        sel.retain(|r| !bad.contains(r));
    }
}

/// What every evaluation of a plan shares.
#[derive(Debug)]
pub struct Shared {
    pub caller: Option<Val>,
    /// The `Fault` constructor of each fault, by `Fault as usize`.
    pub fault_ctors: Vec<u32>,
    pub hosts: Arc<Hosts>,
}

impl Shared {
    pub fn fault_value(&self, f: Fault) -> Val {
        Val::tag(self.fault_ctors[f as usize])
    }
}

/// Returns the rows of `sel` where `value` holds, then the others.
fn split_bool(sel: &Sel, value: impl Fn(u32) -> Result<bool, Error>) -> Result<(Sel, Sel), Error> {
    let mut t = Vec::new();
    let mut f = Vec::new();
    for &r in sel {
        if value(r)? {
            t.push(r);
        } else {
            f.push(r);
        }
    }
    Ok((t, f))
}

impl Code {
    /// Evaluate the expression at the rows of `sel`, removing rows that fault when `catch` is
    /// on. Returns the value at each remaining row, by row.
    pub fn eval(
        &self,
        f: &mut Frame,
        sel: &mut Sel,
        catch: &mut Catch,
        sh: &Shared,
    ) -> Result<Vec<(u32, Val)>, Error> {
        let mut regs = Regs(vec![Vec::new(); self.regs as usize]);
        self.run(&self.block, f, &mut regs, sel, catch, sh)?;
        Ok(sel
            .iter()
            .map(|&r| (r, self.get(self.block.out, f, &regs, r).clone()))
            .collect())
    }

    /// Evaluate the expression at one row of a frame of one row.
    pub fn eval_one(&self, f: &mut Frame, sh: &Shared) -> Result<Val, Error> {
        let mut sel = vec![0];
        let mut v = self.eval(f, &mut sel, &mut Catch::off(), sh)?;
        Ok(v.pop().expect("a row with no catch keeps its value").1)
    }

    fn get<'a>(&'a self, o: Opd, f: &'a Frame, regs: &'a Regs, row: u32) -> &'a Val {
        match o {
            Opd::Var(v) => f.get(v, row),
            Opd::Reg(r) => &regs.0[r as usize][row as usize],
            Opd::Lit(i) => &self.lits[i as usize],
        }
    }

    fn put(regs: &mut Regs, len: usize, dst: u32, vals: Vec<(u32, Val)>) {
        let col = &mut regs.0[dst as usize];
        if col.len() < len {
            col.resize(len, Val::Int(0));
        }
        for (r, v) in vals {
            col[r as usize] = v;
        }
    }

    /// Evaluate `each` at every row of `sel`; drop rows whose evaluation faults.
    fn map<'a>(
        &'a self,
        f: &'a Frame,
        regs: &'a Regs,
        sel: &mut Sel,
        catch: &mut Catch,
        mut each: impl FnMut(&dyn Fn(Opd) -> &'a Val) -> Result<Result<Val, Fault>, Error>,
    ) -> Result<Vec<(u32, Val)>, Error> {
        let since = catch.faults.len();
        let mut out = Vec::with_capacity(sel.len());
        for &r in sel.iter() {
            let get = |o: Opd| self.get(o, f, regs, r);
            match each(&get)? {
                Ok(v) => out.push((r, v)),
                Err(fault) => catch.fault(r, fault)?,
            }
        }
        catch.drop_faulted(since, sel);
        Ok(out)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per instruction; each arm would have to be a function"
    )]
    fn run(
        &self,
        b: &Block,
        f: &mut Frame,
        regs: &mut Regs,
        sel: &mut Sel,
        catch: &mut Catch,
        sh: &Shared,
    ) -> Result<(), Error> {
        let len = f.len();
        for ins in &b.ins {
            if sel.is_empty() {
                return Ok(());
            }
            match ins {
                Ins::Prim { op, args, dst } => {
                    let mut xs: Vec<&Val> = Vec::with_capacity(args.len());
                    let since = catch.faults.len();
                    let mut out = Vec::with_capacity(sel.len());
                    for &r in sel.iter() {
                        xs.clear();
                        xs.extend(args.iter().map(|o| self.get(*o, f, regs, r)));
                        match prim::apply(op, &xs)? {
                            Ok(v) => out.push((r, v)),
                            Err(fault) => catch.fault(r, fault)?,
                        }
                    }
                    drop(xs);
                    catch.drop_faulted(since, sel);
                    Code::put(regs, len, *dst, out);
                }
                Ins::Record { fields, dst } => {
                    let out = self.map(f, regs, sel, catch, |get| {
                        Ok(Ok(Val::record(
                            fields.iter().map(|o| get(*o).clone()).collect(),
                        )))
                    })?;
                    Code::put(regs, len, *dst, out);
                }
                Ins::Ctor { ctor, fields, dst } => {
                    let out = self.map(f, regs, sel, catch, |get| {
                        Ok(Ok(Val::ctor(
                            *ctor,
                            fields.iter().map(|o| get(*o).clone()).collect(),
                        )))
                    })?;
                    Code::put(regs, len, *dst, out);
                }
                Ins::Field { src, at, dst } => {
                    let out = self.map(f, regs, sel, catch, |get| {
                        let v = get(*src);
                        let i = match v {
                            Val::Ctor(..) => at.of(v.ctor_of()?)?,
                            _ => at.of(0)?,
                        };
                        Ok(Ok(v.fields()?[i].clone()))
                    })?;
                    Code::put(regs, len, *dst, out);
                }
                Ins::Key {
                    src,
                    at,
                    single,
                    dst,
                } => {
                    let out = self.map(f, regs, sel, catch, |get| {
                        let v = get(*src);
                        let c = v.ctor_of()?;
                        let fs = v.fields()?;
                        let mut ks: Vec<Val> = at
                            .iter()
                            .map(|a| Ok(fs[a.of(c)?].clone()))
                            .collect::<Result<_, Error>>()?;
                        Ok(Ok(if *single {
                            ks.pop().expect("a key has a field")
                        } else {
                            Val::record(ks)
                        }))
                    })?;
                    Code::put(regs, len, *dst, out);
                }
                Ins::Tag { src, dst } => {
                    let out = self.map(f, regs, sel, catch, |get| {
                        Ok(Ok(Val::tag(get(*src).ctor_of()?)))
                    })?;
                    Code::put(regs, len, *dst, out);
                }
                Ins::Singleton { src, dst } => {
                    let out = self.map(f, regs, sel, catch, |get| {
                        Ok(Ok(Val::set(BTreeSet::from([get(*src).clone()]))))
                    })?;
                    Code::put(regs, len, *dst, out);
                }
                Ins::Union { a, b, dst } => {
                    let out = self.map(f, regs, sel, catch, |get| {
                        let (x, y) = (get(*a).as_set()?, get(*b).as_set()?);
                        Ok(Ok(if y.is_empty() {
                            get(*a).clone()
                        } else if x.is_empty() {
                            get(*b).clone()
                        } else {
                            Val::set(x.union(y).cloned().collect())
                        }))
                    })?;
                    Code::put(regs, len, *dst, out);
                }
                Ins::Restrict { src, keep, dst } => {
                    let out = self.map(f, regs, sel, catch, |get| {
                        let mut kept = BTreeSet::new();
                        for v in get(*src).as_set()? {
                            if keep[v.ctor_of()? as usize] {
                                kept.insert(v.clone());
                            }
                        }
                        Ok(Ok(Val::set(kept)))
                    })?;
                    Code::put(regs, len, *dst, out);
                }
                Ins::Bind { var, src } => {
                    let vals: Vec<(u32, Val)> = sel
                        .iter()
                        .map(|&r| (r, self.get(*src, f, regs, r).clone()))
                        .collect();
                    f.bind(*var, vals);
                }
                Ins::If {
                    cond,
                    then,
                    els,
                    dst,
                } => {
                    let (mut t, mut e) = split_bool(sel, |r| self.get(*cond, f, regs, r).truth())?;
                    let since = catch.faults.len();
                    let mut out = Vec::new();
                    for (blk, s) in [(then, &mut t), (els, &mut e)] {
                        self.run(blk, f, regs, s, catch, sh)?;
                        out.extend(
                            s.iter()
                                .map(|&r| (r, self.get(blk.out, f, regs, r).clone())),
                        );
                    }
                    catch.drop_faulted(since, sel);
                    Code::put(regs, len, *dst, out);
                }
                Ins::Case { scrut, arms, dst } => {
                    let mut by: Vec<Sel> = vec![Vec::new(); arms.len()];
                    for &r in sel.iter() {
                        let c = self.get(*scrut, f, regs, r).ctor_of()?;
                        super::arm(arms, c)?;
                        by[c as usize].push(r);
                    }
                    let since = catch.faults.len();
                    let mut out = Vec::new();
                    for (c, mut s) in by.into_iter().enumerate() {
                        if s.is_empty() {
                            continue;
                        }
                        let (var, blk) = super::arm(arms, c as u32)?;
                        let vals: Vec<(u32, Val)> = s
                            .iter()
                            .map(|&r| (r, self.get(*scrut, f, regs, r).clone()))
                            .collect();
                        f.bind(*var, vals);
                        self.run(blk, f, regs, &mut s, catch, sh)?;
                        out.extend(
                            s.iter()
                                .map(|&r| (r, self.get(blk.out, f, regs, r).clone())),
                        );
                    }
                    catch.drop_faulted(since, sel);
                    Code::put(regs, len, *dst, out);
                }
                Ins::Try { body, dst } => {
                    let mut inner = Catch::on();
                    let mut s = sel.clone();
                    self.run(body, f, regs, &mut s, &mut inner, sh)?;
                    let mut out: Vec<(u32, Val)> = s
                        .iter()
                        .map(|&r| {
                            (
                                r,
                                Val::ctor(0, vec![self.get(body.out, f, regs, r).clone()]),
                            )
                        })
                        .collect();
                    out.extend(
                        inner
                            .faults
                            .iter()
                            .map(|(r, x)| (*r, Val::ctor(1, vec![sh.fault_value(*x)]))),
                    );
                    Code::put(regs, len, *dst, out);
                }
                Ins::Caller { dst } => {
                    let c = sh.caller.clone().ok_or(Error::NoCaller)?;
                    let out = sel.iter().map(|&r| (r, c.clone())).collect();
                    Code::put(regs, len, *dst, out);
                }
                Ins::Host { call, args, dst } => {
                    let rows: Vec<Vec<&Val>> = sel
                        .iter()
                        .map(|&r| args.iter().map(|o| self.get(*o, f, regs, r)).collect())
                        .collect();
                    match call.call(&sh.hosts, &rows)? {
                        Ok(vals) => {
                            let out = sel.iter().copied().zip(vals).collect();
                            Code::put(regs, len, *dst, out);
                        }
                        Err(fault) => {
                            let since = catch.faults.len();
                            for &r in sel.iter() {
                                catch.fault(r, fault)?;
                            }
                            catch.drop_faulted(since, sel);
                        }
                    }
                }
                Ins::Reject { msg } => {
                    if let Some(&r) = sel.first() {
                        let Val::Text(t) = self.get(*msg, f, regs, r) else {
                            return Err(Error::Internal("`reject` takes a Text".into()));
                        };
                        return Err(Error::Rejected(t.to_string()));
                    }
                }
                Ins::Unreachable => {
                    return Err(Error::Internal(
                        "an evaluation reaches no `Unreachable`".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Compiles the scalar expressions of one arena.
pub struct Compiler<'a> {
    schema: &'a Schema,
    terms: &'a Terms,
    codes: HashMap<Tm, Arc<Code>>,
}

/// The state of compiling one expression.
struct Emit {
    regs: u32,
    lits: Vec<Val>,
}

impl Emit {
    fn reg(&mut self) -> u32 {
        self.regs += 1;
        self.regs - 1
    }

    fn lit(&mut self, v: Val) -> Opd {
        self.lits.push(v);
        Opd::Lit(self.lits.len() as u32 - 1)
    }
}

impl<'a> Compiler<'a> {
    pub fn new(schema: &'a Schema, terms: &'a Terms) -> Compiler<'a> {
        Compiler {
            schema,
            terms,
            codes: HashMap::new(),
        }
    }

    pub fn schema(&self) -> &'a Schema {
        self.schema
    }

    pub fn terms(&self) -> &'a Terms {
        self.terms
    }

    /// Returns the code of scalar expression `t`.
    pub fn code(&mut self, t: Tm) -> Result<Arc<Code>, Error> {
        if let Some(c) = self.codes.get(&t) {
            return Ok(c.clone());
        }
        let mut e = Emit {
            regs: 0,
            lits: Vec::new(),
        };
        let block = self.block(&mut e, t)?;
        let code = Arc::new(Code {
            block,
            regs: e.regs,
            lits: e.lits,
        });
        self.codes.insert(t, code.clone());
        Ok(code)
    }

    /// Returns the call of host function `func` with `args` for a result of type `ty`.
    ///
    /// Fails with `Unsupported` for a function of another kind and for one whose argument or
    /// result types no batch column holds.
    fn host(&self, func: FnId, args: &[Tm], ty: &Ty) -> Result<HostCall, Error> {
        let def = self.schema.function(func);
        let unsupported = || Error::Unsupported(format!("a call of `{}`", def.name));
        let FnBody::Host { name, .. } = &def.body else {
            return Err(unsupported());
        };
        let column = |t: &Ty| batch_type(self.schema, t).ok_or_else(unsupported);
        Ok(HostCall {
            function: def.name.clone(),
            symbol: name.clone(),
            params: args
                .iter()
                .map(|x| column(self.terms.ty(*x)))
                .collect::<Result<_, _>>()?,
            result: column(ty)?,
            conv: conv(self.schema, ty),
        })
    }

    fn block(&self, e: &mut Emit, t: Tm) -> Result<Block, Error> {
        let mut ins = Vec::new();
        let out = self.expr(e, &mut ins, t)?;
        Ok(Block { ins, out })
    }

    fn constant(&self, c: &Const, ty: &Ty) -> Result<Val, Error> {
        let b = base(self.schema, ty);
        Ok(match (c, b) {
            (Const::Int(i), Some(Base::Decimal(s))) => Val::Decimal(
                i.checked_mul(10i128.pow(u32::from(s)))
                    .ok_or_else(|| Error::Internal("a decimal literal fits its type".into()))?,
            ),
            (Const::Int(i), Some(Base::F32)) => Val::F32(F32::new(*i as f32)),
            (Const::Int(i), Some(Base::F64)) => Val::F64(F64::new(*i as f64)),
            (Const::Int(i), _) => Val::Int(
                i64::try_from(*i)
                    .map_err(|_| Error::Internal("an integer literal fits its type".into()))?,
            ),
            (Const::Decimal(d), _) => Val::Decimal(*d),
            (Const::Float(x), Some(Base::F32)) => Val::F32(F32::new(*x as f32)),
            (Const::Float(x), _) => Val::F64(F64::new(*x)),
            (Const::Text(s), _) => Val::text(s),
            (Const::Duration(d), _) => Val::Duration(*d),
        })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per term kind; each arm would have to be its own emitter"
    )]
    fn expr(&self, e: &mut Emit, ins: &mut Vec<Ins>, t: Tm) -> Result<Opd, Error> {
        let ty = self.terms.ty(t);
        let kind = self.terms.kind(t);
        Ok(match kind {
            Kind::Var(v) => Opd::Var(v.0),
            Kind::Const(c) => {
                let v = self.constant(c, ty)?;
                e.lit(v)
            }
            Kind::Prim(p, xs) => {
                let tys: Vec<Ty> = xs.iter().map(|x| self.terms.ty(*x).clone()).collect();
                let consts = xs
                    .iter()
                    .map(|x| match self.terms.kind(*x) {
                        Kind::Const(c) => self.constant(c, self.terms.ty(*x)).map(Some),
                        _ => Ok(None),
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                let op = prim::choose(self.schema, *p, &tys, &consts, ty)?;
                let args = xs
                    .iter()
                    .map(|x| self.expr(e, ins, *x))
                    .collect::<Result<_, _>>()?;
                let dst = e.reg();
                ins.push(Ins::Prim { op, args, dst });
                Opd::Reg(dst)
            }
            Kind::Record(fs) => {
                let fields = fs
                    .iter()
                    .map(|(_, x)| self.expr(e, ins, *x))
                    .collect::<Result<_, _>>()?;
                let dst = e.reg();
                ins.push(Ins::Record { fields, dst });
                Opd::Reg(dst)
            }
            Kind::Ctor { ctor, fields } => {
                let given: Vec<(String, Opd)> = fields
                    .iter()
                    .map(|(n, x)| Ok((n.clone(), self.expr(e, ins, *x)?)))
                    .collect::<Result<_, Error>>()?;
                let order = match repr(self.schema, ty) {
                    Ty::Data { .. } => ctors(self.schema, ty)
                        .ok_or_else(|| Error::Internal("a constructor's type is a sum".into()))?
                        .swap_remove(*ctor as usize)
                        .into_iter()
                        .map(|(n, _)| n)
                        .collect(),
                    _ => given.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
                };
                let fields = order
                    .iter()
                    .map(|n| {
                        given
                            .iter()
                            .find(|(g, _)| g == n)
                            .map(|(_, o)| *o)
                            .ok_or_else(|| {
                                Error::Internal(format!("constructor field `{n}` is given"))
                            })
                    })
                    .collect::<Result<_, Error>>()?;
                let dst = e.reg();
                ins.push(Ins::Ctor {
                    ctor: *ctor,
                    fields,
                    dst,
                });
                Opd::Reg(dst)
            }
            Kind::Field(x, name) => {
                let (at, _) = field(self.schema, self.terms.ty(*x), name)?;
                let src = self.expr(e, ins, *x)?;
                let dst = e.reg();
                ins.push(Ins::Field { src, at, dst });
                Opd::Reg(dst)
            }
            Kind::Key(x) => {
                let Ty::Key { rel, .. } = ty else {
                    return Err(Error::Internal(format!(
                        "`key` has a key type, found {ty:?}"
                    )));
                };
                let (at, single) = key_fields(self.schema, *rel, self.terms.ty(*x))?;
                let src = self.expr(e, ins, *x)?;
                let dst = e.reg();
                ins.push(Ins::Key {
                    src,
                    at,
                    single,
                    dst,
                });
                Opd::Reg(dst)
            }
            Kind::KeyOf(_, x) => self.expr(e, ins, *x)?,
            Kind::Tag(x) => {
                let src = self.expr(e, ins, *x)?;
                let dst = e.reg();
                ins.push(Ins::Tag { src, dst });
                Opd::Reg(dst)
            }
            Kind::Singleton(x) => {
                let src = self.expr(e, ins, *x)?;
                let dst = e.reg();
                ins.push(Ins::Singleton { src, dst });
                Opd::Reg(dst)
            }
            Kind::EmptySet => e.lit(Val::empty_set()),
            Kind::Union(a, b) => {
                let a = self.expr(e, ins, *a)?;
                let b = self.expr(e, ins, *b)?;
                let dst = e.reg();
                ins.push(Ins::Union { a, b, dst });
                Opd::Reg(dst)
            }
            Kind::Restrict(x, set) => {
                let elem = super::types::elem(self.schema, self.terms.ty(*x))
                    .ok_or_else(|| Error::Internal("a restriction is of a set".into()))?;
                let n = ctors(self.schema, &elem).map_or(0, |c| c.len());
                let mut keep = vec![false; n];
                for c in set {
                    keep[*c as usize] = true;
                }
                let src = self.expr(e, ins, *x)?;
                let dst = e.reg();
                ins.push(Ins::Restrict { src, keep, dst });
                Opd::Reg(dst)
            }
            Kind::Let(v, a, b) => {
                let src = self.expr(e, ins, *a)?;
                ins.push(Ins::Bind { var: v.0, src });
                self.expr(e, ins, *b)?
            }
            Kind::If(c, a, b) => {
                let cond = self.expr(e, ins, *c)?;
                let then = self.block(e, *a)?;
                let els = self.block(e, *b)?;
                let dst = e.reg();
                ins.push(Ins::If {
                    cond,
                    then,
                    els,
                    dst,
                });
                Opd::Reg(dst)
            }
            Kind::Case { scrutinee, arms } => {
                let sty = self.terms.ty(*scrutinee);
                let n = ctors(self.schema, sty).map_or(0, |c| c.len());
                let scrut = self.expr(e, ins, *scrutinee)?;
                let mut compiled = vec![None; n];
                for (c, v, body) in arms {
                    compiled[*c as usize] = Some((v.0, self.block(e, *body)?));
                }
                let dst = e.reg();
                ins.push(Ins::Case {
                    scrut,
                    arms: compiled,
                    dst,
                });
                Opd::Reg(dst)
            }
            Kind::Try(x) => {
                let body = self.block(e, *x)?;
                let dst = e.reg();
                ins.push(Ins::Try { body, dst });
                Opd::Reg(dst)
            }
            Kind::Caller => {
                let dst = e.reg();
                ins.push(Ins::Caller { dst });
                Opd::Reg(dst)
            }
            Kind::Call { func, args, .. } => {
                let call = self.host(*func, args, ty)?;
                let args = args
                    .iter()
                    .map(|x| self.expr(e, ins, *x))
                    .collect::<Result<_, _>>()?;
                let dst = e.reg();
                ins.push(Ins::Host {
                    call: Box::new(call),
                    args,
                    dst,
                });
                Opd::Reg(dst)
            }
            Kind::Reject(m) => {
                let msg = self.expr(e, ins, *m)?;
                ins.push(Ins::Reject { msg });
                e.lit(Val::Int(0))
            }
            Kind::Unreachable => {
                ins.push(Ins::Unreachable);
                e.lit(Val::Int(0))
            }
            other => {
                return Err(Error::Unsupported(format!(
                    "a scalar expression of the form {other:?}"
                )));
            }
        })
    }
}
