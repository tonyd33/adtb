use super::expr::wants_expected;
use super::*;
use crate::ast::{CallArgs, Expr};
use crate::kernel::{Agg, FileFormat, Kind, Prim};

impl Checker<'_> {
    /// Checks arguments against parameter types over `n` type parameters of a callee, solving those parameters from the synthesized arguments, then from `result` against `expected`.
    pub(super) fn solve_args(
        &mut self,
        params: &[Ty],
        es: &[&Expr],
        n: usize,
        result: Option<(&Ty, &Ty)>,
    ) -> R<(Vec<Tm>, Vec<Option<Ty>>)> {
        let mut sol: Vec<Option<Ty>> = vec![None; n];
        let mut outs: Vec<Option<Out>> = vec![None; es.len()];
        if n > 0 {
            for (i, (e, pty)) in es.iter().zip(params).enumerate() {
                if !wants_expected(e) && mentions_any_param(pty, n) {
                    let o = self.synth(e)?;
                    unify(self, pty, &o.ty, &mut sol);
                    outs[i] = Some(o);
                }
            }
            if let Some((res, exp)) = result
                && sol.iter().any(Option::is_none)
            {
                let mut from_result = vec![None; n];
                unify(self, res, exp, &mut from_result);
                for (s, r) in sol.iter_mut().zip(from_result) {
                    if s.is_none() {
                        *s = r;
                    }
                }
            }
        }
        let mut tms = Vec::new();
        for (i, (e, pty)) in es.iter().zip(params).enumerate() {
            let o = match outs[i].take() {
                Some(o) => {
                    let want = pty.subst(&solved(&sol));
                    self.subsume(o, &want, e.span)?
                }
                None if (0..n).any(|j| sol[j].is_none() && mentions(pty, j as u32)) => {
                    let o = self.synth(e)?;
                    unify(self, pty, &o.ty, &mut sol);
                    o
                }
                None => {
                    let want = pty.subst(&solved(&sol));
                    self.check(e, &want)?
                }
            };
            tms.push(o.tm);
        }
        Ok((tms, sol))
    }

    pub(super) fn lambda(
        &mut self,
        ps: &ast::LambdaParams,
        body: &Expr,
        expected: Option<&Ty>,
        span: Span,
    ) -> R<Out> {
        self.scope.push();
        let r = self.lambda_in(ps, body, expected, span);
        self.scope.pop();
        r
    }

    fn lambda_in(
        &mut self,
        ps: &ast::LambdaParams,
        body: &Expr,
        expected: Option<&Ty>,
        span: Span,
    ) -> R<Out> {
        match ps {
            ast::LambdaParams::One(x) => {
                let Some(Ty::Fn(a, r)) = expected else {
                    return Err(diag(
                        span,
                        "the parameter type of `fn x -> ...` is not known here; write `fn (x: T) -> ...`",
                    ));
                };
                let v = self.bind(&x.text, (**a).clone(), None);
                let b = self.check(body, r)?;
                let ty = Ty::Fn(a.clone(), r.clone());
                Ok(self.out(Kind::Lam(v, b.tm), ty))
            }
            ast::LambdaParams::List(params) => {
                let mut vars = Vec::new();
                let mut want = expected.cloned();
                for p in params {
                    let t = self.resolve_type(&p.ty)?;
                    if let Some(Ty::Fn(a, r)) = &want {
                        if !subtype(self.schema, a, &t) {
                            return Err(diag(
                                p.ty.span,
                                format!(
                                    "expected a parameter of {}, found {}",
                                    self.show(a),
                                    self.show(&t)
                                ),
                            ));
                        }
                        want = Some((**r).clone());
                    } else {
                        want = None;
                    }
                    vars.push(self.bind(&p.name.text, t, None));
                }
                let b = match &want {
                    Some(r) => self.check(body, r)?,
                    None => self.synth(body)?,
                };
                let tm = self.lams(vars, b.tm, b.ty);
                let ty = self.terms.ty(tm).clone();
                Ok(Out { tm, ty, rows: None })
            }
        }
    }

    pub(super) fn call(
        &mut self,
        callee: &ast::QName,
        args: &CallArgs,
        pins: Option<&ast::Version>,
        expected: Option<&Ty>,
        span: Span,
    ) -> R<Out> {
        let text = callee.name.text.as_str();
        let format = match text {
            "read_csv" => Some(FileFormat::Csv),
            "read_ndjson" => Some(FileFormat::Ndjson),
            "read_parquet" => Some(FileFormat::Parquet),
            "read_arrow" => Some(FileFormat::Arrow),
            _ => None,
        };
        if let Some(v) = pins
            && (format.is_none() || callee.module.is_some())
        {
            return Err(diag(v.span, "only a file scan takes `pins`"));
        }
        if text.starts_with(|c: char| c.is_ascii_uppercase()) {
            return self.newtype_call(callee, args, span);
        }
        if callee.module.is_none()
            && let Some(local) = self.scope.lookup(text).cloned()
        {
            return self.apply_local(local, args, span);
        }
        match self.lookup_value(callee) {
            Ok(ValueDef::Function(f)) => return self.call_fn(f, args, expected, span),
            Ok(ValueDef::Aggregate(a)) => {
                return match args {
                    CallArgs::Aggregate {
                        expr,
                        generators,
                        filter,
                    } => {
                        self.scope.push();
                        let r = self.aggregate_in(
                            Agg::User(a),
                            text,
                            expr,
                            generators,
                            filter.as_deref(),
                        );
                        self.scope.pop();
                        r
                    }
                    CallArgs::List(_) => Err(diag(
                        span,
                        format!("`{text}` is an aggregate: `{text}(e for x in q)`"),
                    )),
                };
            }
            Ok(ValueDef::Relation(_)) => {
                return Err(diag(
                    span,
                    format!("`{text}` is a relation, not a function"),
                ));
            }
            Ok(_) => return Err(diag(span, format!("`{text}` is not a function"))),
            Err(e) => {
                if callee.module.is_some() || !PRELUDE_FUNCTIONS.contains(&text) {
                    return Err(diag(callee.span(), e));
                }
            }
        }
        match (args, format) {
            (
                CallArgs::Aggregate {
                    expr,
                    generators,
                    filter,
                },
                _,
            ) => self.aggregate(text, expr, generators, filter.as_deref(), span),
            (CallArgs::List(es), Some(format)) => self.file_scan(text, format, es, pins, span),
            (CallArgs::List(es), None) => self.prelude_call(text, es, span),
        }
    }

    fn newtype_call(&mut self, callee: &ast::QName, args: &CallArgs, span: Span) -> R<Out> {
        let def = match self.lookup_type(callee) {
            Ok(TypeName::Decl(d)) => d,
            Ok(_) => {
                return Err(diag(
                    callee.span(),
                    format!("`{}` is not a newtype", callee.name.text),
                ));
            }
            Err(e) => return Err(diag(callee.span(), e)),
        };
        self.note(callee.name.span, Ref::Type(def));
        let TypeDefKind::Newtype { base, .. } = &self.schema.ty(def).kind else {
            return Err(diag(
                callee.span(),
                format!(
                    "`{}` is not a newtype; build a constructor with `{{ ... }}`",
                    callee.name.text
                ),
            ));
        };
        let base = base.clone();
        let CallArgs::List(es) = args else {
            return Err(diag(span, "a newtype takes one value"));
        };
        let [e] = es.as_slice() else {
            return Err(diag(span, "a newtype takes one value"));
        };
        let o = self.check(e, &base)?;
        let ty = Ty::Data {
            def,
            args: Vec::new(),
            ctors: None,
        };
        Ok(self.out(Kind::Prim(Prim::Wrap, vec![o.tm]), ty))
    }

    fn apply_local(&mut self, local: Local, args: &CallArgs, span: Span) -> R<Out> {
        let CallArgs::List(es) = args else {
            return Err(diag(
                span,
                "an aggregate form applies to an aggregate, not a function value",
            ));
        };
        let mut ty = local.ty.clone();
        let mut f = self.var_tm(local.var);
        if es.is_empty() {
            match ty {
                Ty::Fn(a, r) if *a == Ty::Unit => {
                    let unit = self.add(Kind::Record(Vec::new()), Ty::Unit);
                    f = self.add(Kind::App(f, unit), (*r).clone());
                    ty = *r;
                }
                _ => {
                    return Err(diag(
                        span,
                        format!("{} is not a function of no arguments", self.show(&local.ty)),
                    ));
                }
            }
        }
        for e in es {
            let Ty::Fn(a, r) = ty else {
                return Err(diag(
                    e.span,
                    format!("too many arguments for {}", self.show(&local.ty)),
                ));
            };
            let x = self.check(e, &a)?.tm;
            f = self.add(Kind::App(f, x), (*r).clone());
            ty = *r;
        }
        Ok(Out {
            tm: f,
            ty,
            rows: None,
        })
    }

    fn call_fn(&mut self, f: FnId, args: &CallArgs, expected: Option<&Ty>, span: Span) -> R<Out> {
        let def = self.schema.function(f).clone();
        let CallArgs::List(es) = args else {
            return Err(diag(
                span,
                format!("`{}` is a function, not an aggregate", def.name),
            ));
        };
        if es.len() != def.params.len() {
            return Err(diag(
                span,
                format!(
                    "`{}` takes {} arguments, given {}",
                    def.name,
                    def.params.len(),
                    es.len()
                ),
            ));
        }
        let params: Vec<Ty> = def.params.iter().map(|(_, t)| t.clone()).collect();
        let es: Vec<&Expr> = es.iter().collect();
        let n = def.type_params.len();
        let (tms, sol) = self.solve_args(&params, &es, n, expected.map(|e| (&def.result, e)))?;
        let mut type_args = Vec::new();
        for (j, (name, bounds)) in def.type_params.iter().enumerate() {
            let Some(t) = sol[j].clone() else {
                return Err(diag(
                    span,
                    format!(
                        "the type argument `{name}` of `{}` is not known here",
                        def.name
                    ),
                ));
            };
            if t.mentions_fn() {
                return Err(diag(
                    span,
                    "a type argument is a value type, never a function type",
                ));
            }
            for b in bounds {
                let ok = match b {
                    Bound::Ord => self.ordered(&t),
                    Bound::Num => self.is_num(&t),
                    Bound::Eq | Bound::Hash => true,
                };
                if !ok {
                    return Err(diag(
                        span,
                        format!(
                            "{} does not satisfy the bound `{name}: {b:?}` of `{}`",
                            self.show(&t),
                            def.name
                        ),
                    ));
                }
            }
            type_args.push(t);
        }
        let result = def.result.subst(&type_args);
        let mut o = self.out(
            Kind::Call {
                func: f,
                type_args,
                args: tms,
            },
            result,
        );
        o.rows = self.fn_rows[f.0 as usize];
        Ok(o)
    }
}

fn mentions_any_param(t: &Ty, n: usize) -> bool {
    (0..n as u32).any(|i| mentions(t, i))
}

fn solved(sol: &[Option<Ty>]) -> Vec<Ty> {
    sol.iter()
        .enumerate()
        .map(|(i, t)| t.clone().unwrap_or(Ty::Param(i as u32)))
        .collect()
}

pub(super) fn mentions(t: &Ty, i: u32) -> bool {
    match t {
        Ty::Param(j) => *j == i,
        Ty::Data { args, .. } => args.iter().any(|a| mentions(a, i)),
        Ty::Set(x) | Ty::NonEmpty(x) | Ty::Tag(x) => mentions(x, i),
        Ty::Group(a, b) | Ty::Fn(a, b) => mentions(a, i) || mentions(b, i),
        Ty::Record(fs) => fs.iter().any(|(_, x)| mentions(x, i)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .any(|(_, fs)| fs.iter().any(|(_, x)| mentions(x, i))),
        _ => false,
    }
}

/// Collects lower bounds for the callee's type parameters in `pattern` from `actual`.
fn unify(c: &Checker, pattern: &Ty, actual: &Ty, sol: &mut [Option<Ty>]) {
    match (pattern, actual) {
        (Ty::Param(i), t) => {
            let i = *i as usize;
            if i < sol.len() {
                sol[i] = match &sol[i] {
                    None => Some(t.clone()),
                    Some(s) => Some(c.join(s, t).unwrap_or_else(|| s.clone())),
                };
            }
        }
        (
            Ty::Data {
                def: d1, args: a1, ..
            },
            Ty::Data {
                def: d2, args: a2, ..
            },
        ) if d1 == d2 => {
            for (p, a) in a1.iter().zip(a2) {
                unify(c, p, a, sol);
            }
        }
        (Ty::Set(p) | Ty::NonEmpty(p), Ty::Set(a) | Ty::NonEmpty(a)) | (Ty::Tag(p), Ty::Tag(a)) => {
            unify(c, p, a, sol);
        }
        (Ty::Fn(p1, r1), Ty::Fn(p2, r2)) => {
            unify(c, p1, p2, sol);
            unify(c, r1, r2, sol);
        }
        (Ty::Group(k1, v1), Ty::Group(k2, v2)) => {
            unify(c, k1, k2, sol);
            unify(c, v1, v2, sol);
        }
        (Ty::Record(fs), Ty::Record(gs)) => {
            for (n, p) in fs {
                if let Some((_, a)) = gs.iter().find(|(m, _)| m == n) {
                    unify(c, p, a, sol);
                }
            }
        }
        _ => {}
    }
}
