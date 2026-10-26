use super::*;
use crate::ast::{Arm, Expr, ExprKind, Lit, Pattern, PatternKind, Payload, UnOp};
use crate::kernel::{Kind, Prim};

/// A pattern elaborated against the type of the value it matches.
#[derive(Clone, Debug)]
enum Pat {
    /// `_`, a binder, or a field a record pattern leaves out.
    Wild {
        binder: Option<String>,
        span: Span,
    },
    Lit {
        tm: Tm,
        span: Span,
    },
    /// A constructor, or every member of a named set.
    Ctor {
        set: CtorSet,
        binder: Option<String>,
        fields: Vec<(String, Pat)>,
        span: Span,
    },
    Or(Vec<Pat>),
}

impl Pat {
    fn wild(span: Span) -> Pat {
        Pat::Wild { binder: None, span }
    }

    fn span(&self) -> Span {
        match self {
            Pat::Wild { span, .. } | Pat::Lit { span, .. } | Pat::Ctor { span, .. } => *span,
            Pat::Or(alts) => alts[0].span(),
        }
    }

    /// The alternatives of an or-pattern, flattened; the pattern itself otherwise.
    fn alts(&self) -> Vec<&Pat> {
        match self {
            Pat::Or(alts) => alts.iter().flat_map(Pat::alts).collect(),
            p => vec![p],
        }
    }
}

/// A value no arm covers, printed as a pattern.
enum Witness {
    Any,
    Ctor {
        name: String,
        fields: Vec<(String, Witness)>,
    },
}

impl Witness {
    fn show(&self) -> String {
        match self {
            Witness::Any => "_".to_string(),
            Witness::Ctor { name, fields } if fields.is_empty() => name.clone(),
            Witness::Ctor { name, fields } => {
                let shown: Vec<String> = fields
                    .iter()
                    .filter(|(_, w)| !matches!(w, Witness::Any))
                    .map(|(f, w)| format!("{f}: {}", w.show()))
                    .collect();
                if shown.is_empty() {
                    format!("{name} _")
                } else {
                    format!("{name} {{ {} }}", shown.join(", "))
                }
            }
        }
    }
}

enum Head {
    Ctor(u32),
    Lit(Tm),
}

struct Cx<'b> {
    binds: &'b BTreeMap<String, Var>,
    res: &'b Ty,
}

/// Where matching continues: `k` on a match, `f` otherwise.
#[derive(Clone, Copy)]
struct Cont {
    k: Tm,
    f: Tm,
}

pub(super) struct ArmIr {
    pat: Pat,
    binds: BTreeMap<String, Var>,
    pub(super) locals: Vec<(String, Local)>,
    pub(super) guard: Option<Tm>,
    pub(super) body: Option<Out>,
}

/// A `match` whose patterns and coverage are checked and whose arm bodies are not yet.
pub(super) struct Matched {
    pub(super) scrutinee: Out,
    sty: Ty,
    pub(super) arms: Vec<ArmIr>,
}

/// A binder a pattern introduces, before the alternatives of its arm are joined.
struct Bound {
    name: String,
    ty: Ty,
    rows: Option<RelId>,
    span: Span,
}

impl Checker<'_> {
    /// The constructors of a sum position, by index; `None` for a scalar.
    fn signature(&self, ty: &Ty) -> Option<Vec<(u32, String)>> {
        match ty {
            Ty::Data { def, .. }
                if !matches!(self.schema.ty(*def).kind, TypeDefKind::Sum { .. }) =>
            {
                None
            }
            Ty::Data { .. } | Ty::Sum { .. } | Ty::Tag(_) => ty::ctor_names(self.schema, ty),
            _ => None,
        }
    }

    /// The fields of constructor `c` of a sum position, header first.
    fn ctor_cols(&self, ty: &Ty, c: u32) -> Vec<(String, Ty)> {
        ty::fields_of_ctor(self.schema, ty, c)
            .into_iter()
            .map(|(n, t, _)| (n, t))
            .collect()
    }

    fn lit_eq(&self, a: Tm, b: Tm) -> bool {
        match (self.terms.kind(a), self.terms.kind(b)) {
            (Kind::Const(x), Kind::Const(y)) => x == y,
            _ => a == b,
        }
    }

    /// Elaborates a `match`: checks its patterns, coverage and arms, and compiles it to a `case` tree.
    pub(super) fn match_expr(
        &mut self,
        scrutinee: &Expr,
        arms: &[Arm],
        expected: Option<&Ty>,
        span: Span,
    ) -> R<Out> {
        let heads: Vec<(&[Pattern], bool)> = arms
            .iter()
            .map(|a| (a.patterns.as_slice(), a.guard.is_some()))
            .collect();
        let mut m = self.match_patterns(scrutinee, &heads, span)?;
        let ty = self.arm_bodies(arms, &mut m.arms, expected)?;
        Ok(self.match_compile(m, ty))
    }

    /// Checks a match's scrutinee, the patterns of each arm given with whether it has a guard, and coverage.
    pub(super) fn match_patterns(
        &mut self,
        scrutinee: &Expr,
        arms: &[(&[Pattern], bool)],
        span: Span,
    ) -> R<Matched> {
        let s = self.synth(scrutinee)?;
        self.note(span, Ref::Match(s.ty.clone()));
        let (s, by_key) = match &s.ty {
            Ty::Key { rel, ctors } => {
                let (rel, ctors) = (*rel, ctors.clone());
                (self.deref(s, rel, ctors), true)
            }
            Ty::Set(_) | Ty::NonEmpty(_) | Ty::Group(..) => {
                return Err(diag(
                    scrutinee.span,
                    format!(
                        "a match examines one value, found {}; a set is ranged over",
                        self.show(&s.ty)
                    ),
                ));
            }
            Ty::Fn(..) | Ty::Change | Ty::Unit => {
                return Err(diag(
                    scrutinee.span,
                    format!("a match does not examine {}", self.show(&s.ty)),
                ));
            }
            _ => (s, false),
        };
        let sty = s.ty.clone();
        let is_sum = self.signature(&sty).is_some();
        let mut irs = Vec::new();
        for (patterns, _) in arms {
            irs.push(self.arm_patterns(patterns, &sty, s.rows, is_sum, by_key)?);
        }
        if is_sum && !self.adhoc() {
            let rows: Vec<Vec<Pat>> = irs.iter().map(|a| vec![a.pat.clone()]).collect();
            self.wild_rule(&rows, std::slice::from_ref(&sty))?;
        }
        let mut covered: Vec<Vec<Pat>> = Vec::new();
        for ((_, guarded), ir) in arms.iter().zip(&irs) {
            let alts = ir.pat.alts();
            let mut seen = covered.clone();
            for alt in &alts {
                let q = vec![(*alt).clone()];
                if self.useful(&seen, &q, std::slice::from_ref(&sty)).is_none() {
                    return Err(diag(
                        alt.span(),
                        if alts.len() == 1 {
                            "this arm is unreachable: the arms before it cover every value it matches"
                        } else {
                            "this alternative is unreachable: the arms and alternatives before it cover every value it matches"
                        },
                    ));
                }
                seen.push(q);
            }
            if !guarded {
                covered.push(vec![ir.pat.clone()]);
            }
        }
        let wild = vec![Pat::wild(span)];
        if self.coverage
            && let Some(w) = self.useful(&covered, &wild, std::slice::from_ref(&sty))
        {
            return Err(match (&w[0], is_sum) {
                (Witness::Any, false) => diag(
                    span,
                    format!(
                        "a match on {} does not cover every value: literal patterns never cover a scalar, so end it with `_` or a binder",
                        self.show(&sty)
                    ),
                ),
                (w, _) => diag(span, format!("match does not cover `{}`", w.show())),
            });
        }
        Ok(Matched {
            scrutinee: s,
            sty,
            arms: irs,
        })
    }

    /// Compiles a match whose arm bodies are checked, at type `ty`, to a `case` tree.
    pub(super) fn match_compile(&mut self, m: Matched, ty: Ty) -> Out {
        let Matched {
            scrutinee: s,
            sty,
            arms: irs,
        } = m;
        let rows = {
            let mut rs = irs.iter().map(|a| a.body.as_ref().expect("checked").rows);
            let first = rs.next().flatten();
            first.filter(|r| rs.all(|x| x == Some(*r)))
        };
        let xi = self.fresh(sty.clone());
        let body = match self.signature(&sty) {
            Some(sig) => {
                let mut case_arms = Vec::new();
                for (c, _) in sig {
                    let row = ty::restrict(self.schema, &sty, vec![c]);
                    let x = self.fresh(row.clone());
                    let xt = self.var_tm(x);
                    let m = self.chain(Some(c), xt, &row, &irs, &ty);
                    case_arms.push((c, x, m));
                }
                let xt = self.var_tm(xi);
                self.add(
                    Kind::Case {
                        scrutinee: xt,
                        arms: case_arms,
                    },
                    ty.clone(),
                )
            }
            None => {
                let xt = self.var_tm(xi);
                self.chain(None, xt, &sty, &irs, &ty)
            }
        };
        let mut o = self.out(Kind::Let(xi, s.tm, body), ty);
        o.rows = rows;
        o
    }

    /// Elaborates an arm's alternatives and joins the types of the names they bind.
    fn arm_patterns(
        &mut self,
        patterns: &[Pattern],
        sty: &Ty,
        rows: Option<RelId>,
        is_sum: bool,
        by_key: bool,
    ) -> R<ArmIr> {
        let mut alts = Vec::new();
        let mut bound: Vec<Vec<Bound>> = Vec::new();
        for p in patterns {
            let mut bs = Vec::new();
            alts.push(self.pattern(p, sty, rows, &mut bs)?);
            bound.push(bs);
            let named = matches!(p.kind, PatternKind::Ctor { .. });
            if by_key && !named {
                return Err(diag(
                    p.span,
                    "a match on a key matches the referenced row by its constructor; `_` and a binder are not patterns for a key",
                ));
            }
            if is_sum && !named && !self.adhoc() {
                return Err(diag(
                    p.span,
                    format!(
                        "`_` and a binder do not match a value of a sum; name the constructors of {}",
                        self.show(sty)
                    ),
                ));
            }
        }
        let first: Vec<&str> = bound[0].iter().map(|b| b.name.as_str()).collect();
        for (bs, p) in bound.iter().zip(patterns).skip(1) {
            let mut names: Vec<&str> = bs.iter().map(|b| b.name.as_str()).collect();
            let mut want = first.clone();
            names.sort_unstable();
            want.sort_unstable();
            if names != want {
                return Err(diag(
                    p.span,
                    "every alternative of an or-pattern binds the same names",
                ));
            }
        }
        let mut binds = BTreeMap::new();
        let mut locals = Vec::new();
        for b in &bound[0] {
            let mut ty = b.ty.clone();
            let mut rows = b.rows;
            for other in bound
                .iter()
                .skip(1)
                .flat_map(|bs| bs.iter().filter(|o| o.name == b.name))
            {
                ty = self.join(&ty, &other.ty).ok_or_else(|| {
                    diag(
                        other.span,
                        format!(
                            "`{}` has no common type across the alternatives: {} and {}",
                            b.name,
                            self.show(&ty),
                            self.show(&other.ty)
                        ),
                    )
                })?;
                if rows != other.rows {
                    rows = None;
                }
            }
            let var = self.terms.var(&b.name, ty.clone());
            binds.insert(b.name.clone(), var);
            locals.push((
                b.name.clone(),
                Local {
                    var,
                    ty,
                    rows,
                    group: None,
                },
            ));
        }
        let pat = if alts.len() == 1 {
            alts.pop().expect("one alternative")
        } else {
            Pat::Or(alts)
        };
        Ok(ArmIr {
            pat,
            binds,
            locals,
            guard: None,
            body: None,
        })
    }

    /// Elaborates `p` against a value of `ty`; `rows` is the relation of that value when `p` is at the top.
    fn pattern(
        &mut self,
        p: &Pattern,
        ty: &Ty,
        rows: Option<RelId>,
        out: &mut Vec<Bound>,
    ) -> R<Pat> {
        let bind =
            |name: &ast::Ident, ty: Ty, rows: Option<RelId>, out: &mut Vec<Bound>| -> R<()> {
                if out.iter().any(|b| b.name == name.text) {
                    return Err(diag(
                        name.span,
                        format!("`{}` is bound twice in one pattern", name.text),
                    ));
                }
                out.push(Bound {
                    name: name.text.clone(),
                    ty,
                    rows,
                    span: name.span,
                });
                Ok(())
            };
        match &p.kind {
            PatternKind::Wildcard => Ok(Pat::wild(p.span)),
            PatternKind::Bind(x) => {
                bind(x, ty.clone(), rows, out)?;
                Ok(Pat::Wild {
                    binder: Some(x.text.clone()),
                    span: p.span,
                })
            }
            PatternKind::Lit { neg, lit } => {
                if self.signature(ty).is_some() {
                    return Err(diag(
                        p.span,
                        format!(
                            "a literal pattern matches a scalar; {} is matched by its constructors",
                            self.show(ty)
                        ),
                    ));
                }
                let e = Expr {
                    kind: ExprKind::Lit(lit.clone()),
                    span: p.span,
                };
                let e = if *neg {
                    Expr {
                        kind: ExprKind::Unary(UnOp::Neg, Box::new(e)),
                        span: p.span,
                    }
                } else {
                    e
                };
                if matches!(lit, Lit::Bool(_)) {
                    return Err(diag(p.span, "`true` and `false` are constructor patterns"));
                }
                let o = self.check(&e, ty)?;
                Ok(Pat::Lit {
                    tm: o.tm,
                    span: p.span,
                })
            }
            PatternKind::Ctor {
                path,
                ctor,
                payload,
            } => {
                if matches!(ty, Ty::Key { .. }) {
                    return Err(diag(
                        p.span,
                        "a nested pattern does not follow a reference; bind the key and match it in the arm",
                    ));
                }
                let name = ctor.as_ref().unwrap_or(&path.name);
                let set = self.pattern_ctor(ty, path, ctor.as_ref(), p.span)?;
                let def = match ty {
                    Ty::Data { def, .. } => Some(*def),
                    _ => None,
                };
                if let Some(d) = def {
                    if ctor.is_some() {
                        self.note(path.name.span, Ref::Type(d));
                    }
                    match self.schema.ty(d).ctor_index(&name.text) {
                        Some(i) => self.note(name.span, Ref::Ctor(d, i)),
                        None => self.note_sel(d, rows, &ast::CtorSel::One(name.clone())),
                    }
                } else {
                    self.note(name.span, Ref::Anonymous);
                }
                let row = ty::restrict(self.schema, ty, set.clone());
                let cols: Vec<(String, Ty)> = if set.len() == 1 {
                    self.ctor_cols(ty, set[0])
                } else {
                    self.fields_of(&row)
                };
                let mut binder = None;
                let mut fields = Vec::new();
                match payload {
                    None => {
                        if set.iter().any(|&c| !self.ctor_cols(ty, c).is_empty()) {
                            return Err(diag(
                                p.span,
                                format!(
                                    "`{}` has fields; match any of its values with `{} _`",
                                    name.text, name.text
                                ),
                            ));
                        }
                    }
                    Some(Payload::Wildcard) => {}
                    Some(Payload::Bind(x)) => {
                        bind(x, row, rows, out)?;
                        binder = Some(x.text.clone());
                    }
                    Some(Payload::Record(fs)) => {
                        for f in fs {
                            let Some((_, fty)) = cols.iter().find(|(n, _)| *n == f.field.text)
                            else {
                                return Err(diag(
                                    f.field.span,
                                    format!("{} has no field `{}`", self.show(&row), f.field.text),
                                ));
                            };
                            if fields.iter().any(|(n, _)| *n == f.field.text) {
                                return Err(diag(
                                    f.field.span,
                                    format!("field `{}` is matched twice", f.field.text),
                                ));
                            }
                            if let Some(d) = def {
                                let pun = f.pattern.is_none();
                                self.note_field(f.field.span, d, Some(&set), &f.field.text, pun);
                            }
                            let sub = match &f.pattern {
                                None => {
                                    bind(&f.field, fty.clone(), None, out)?;
                                    Pat::Wild {
                                        binder: Some(f.field.text.clone()),
                                        span: f.field.span,
                                    }
                                }
                                Some(fp) => self.pattern(fp, fty, None, out)?,
                            };
                            fields.push((f.field.text.clone(), sub));
                        }
                    }
                }
                Ok(Pat::Ctor {
                    set,
                    binder,
                    fields,
                    span: p.span,
                })
            }
        }
    }

    /// Resolves a constructor or named set written in a pattern against the type of the value it matches.
    fn pattern_ctor(
        &self,
        ty: &Ty,
        path: &ast::QName,
        ctor: Option<&ast::Ident>,
        span: Span,
    ) -> R<CtorSet> {
        let name = ctor.unwrap_or(&path.name);
        let Some(sig) = self.signature(ty) else {
            return Err(diag(
                span,
                format!(
                    "`{}` is a constructor pattern, but {} has no constructors; match it with literals, `_` or a binder",
                    name.text,
                    self.show(ty)
                ),
            ));
        };
        let not_ctor = || {
            diag(
                span,
                format!("`{}` is not a constructor of {}", name.text, self.show(ty)),
            )
        };
        let def = match ty {
            Ty::Data { def, .. } => Some(*def),
            Ty::Tag(inner) => match inner.as_ref() {
                Ty::Data { def, .. } => Some(*def),
                _ => None,
            },
            _ => None,
        };
        if ctor.is_some() {
            match (self.lookup_type(path), def) {
                (Ok(TypeName::Decl(d)), Some(own)) if d == own => {}
                _ => return Err(not_ctor()),
            }
        }
        let set: CtorSet = match ty {
            Ty::Data { def, .. } => {
                let t = self.schema.ty(*def);
                match t.ctor_index(&name.text) {
                    Some(i) => vec![i],
                    None => self
                        .schema
                        .type_sel(*def, &ast::CtorSel::One(name.clone()))
                        .map_err(|_| not_ctor())?,
                }
            }
            Ty::Tag(_) => {
                let t = self.schema.ty(def.expect("a tag of a declared sum"));
                vec![t.ctor_index(&name.text).ok_or_else(not_ctor)?]
            }
            Ty::Sum { ctors, .. } => vec![
                ctors
                    .iter()
                    .position(|(c, _)| *c == name.text)
                    .ok_or_else(not_ctor)? as u32,
            ],
            _ => return Err(not_ctor()),
        };
        if let Some(&c) = set.iter().find(|c| !sig.iter().any(|(i, _)| i == *c)) {
            let cname = match ty {
                Ty::Sum { ctors, .. } => ctors[c as usize].0.clone(),
                _ => self.schema.ty(def.expect("declared")).ctors()[c as usize]
                    .name
                    .clone(),
            };
            return Err(diag(
                span,
                format!(
                    "`{cname}` is not among the constructors of {}",
                    self.show(ty)
                ),
            ));
        }
        Ok(set)
    }

    /// Refuses `_` or a binder at a sum position where another arm names a constructor.
    fn wild_rule(&self, rows: &[Vec<Pat>], tys: &[Ty]) -> R<()> {
        let Some((ty, rest_tys)) = tys.split_first() else {
            return Ok(());
        };
        if rows.is_empty() {
            return Ok(());
        }
        let heads: Vec<&Pat> = rows.iter().flat_map(|r| r[0].alts()).collect();
        let sig = self.signature(ty);
        let names = heads.iter().any(|p| matches!(p, Pat::Ctor { .. }));
        match sig {
            Some(sig) if names => {
                if let Some(w) = heads.iter().find(|p| matches!(p, Pat::Wild { .. })) {
                    return Err(diag(
                        w.span(),
                        format!(
                            "`_` and a binder do not stand for constructors of {}; another arm names a constructor here, so every arm does",
                            self.show(ty)
                        ),
                    ));
                }
                for (c, _) in sig {
                    let cols = self.ctor_cols(ty, c);
                    let spec = self.specialize(rows, &Head::Ctor(c), &cols);
                    let mut tys: Vec<Ty> = cols.into_iter().map(|(_, t)| t).collect();
                    tys.extend(rest_tys.iter().cloned());
                    self.wild_rule(&spec, &tys)?;
                }
                Ok(())
            }
            _ => {
                let next: Vec<Vec<Pat>> = rows.iter().map(|r| r[1..].to_vec()).collect();
                self.wild_rule(&next, rest_tys)
            }
        }
    }

    /// Rows of `rows` whose head matches `head`, the head replaced by its fields.
    fn specialize(&self, rows: &[Vec<Pat>], head: &Head, cols: &[(String, Ty)]) -> Vec<Vec<Pat>> {
        let mut out = Vec::new();
        for row in rows {
            for p in row[0].alts() {
                let fields: Option<Vec<Pat>> = match (p, head) {
                    (Pat::Wild { span, .. }, _) => {
                        Some(cols.iter().map(|_| Pat::wild(*span)).collect())
                    }
                    (
                        Pat::Ctor {
                            set, fields, span, ..
                        },
                        Head::Ctor(c),
                    ) if set.contains(c) => Some(
                        cols.iter()
                            .map(|(n, _)| {
                                fields
                                    .iter()
                                    .find(|(f, _)| f == n)
                                    .map(|(_, p)| p.clone())
                                    .unwrap_or(Pat::wild(*span))
                            })
                            .collect(),
                    ),
                    (Pat::Lit { tm, .. }, Head::Lit(t)) if self.lit_eq(*tm, *t) => Some(Vec::new()),
                    _ => None,
                };
                if let Some(mut fs) = fields {
                    fs.extend(row[1..].iter().cloned());
                    out.push(fs);
                }
            }
        }
        out
    }

    /// Returns a value `q` matches that no row of `rows` matches, if one exists.
    fn useful(&self, rows: &[Vec<Pat>], q: &[Pat], tys: &[Ty]) -> Option<Vec<Witness>> {
        let Some((head, rest)) = q.split_first() else {
            return rows.is_empty().then(Vec::new);
        };
        let ty = &tys[0];
        let with_cols = |cols: &[(String, Ty)]| -> Vec<Ty> {
            cols.iter()
                .map(|(_, t)| t.clone())
                .chain(tys[1..].iter().cloned())
                .collect()
        };
        let ctor_name = |c: u32| -> String {
            self.signature(ty)
                .and_then(|s| s.into_iter().find(|(i, _)| *i == c))
                .map(|(_, n)| n)
                .unwrap_or_default()
        };
        let rebuild = |c: u32, cols: &[(String, Ty)], mut w: Vec<Witness>| -> Vec<Witness> {
            let rest = w.split_off(cols.len());
            let fields = cols.iter().map(|(n, _)| n.clone()).zip(w).collect();
            let mut out = vec![Witness::Ctor {
                name: ctor_name(c),
                fields,
            }];
            out.extend(rest);
            out
        };
        match head {
            Pat::Or(alts) => alts.iter().find_map(|a| {
                let mut q2 = vec![a.clone()];
                q2.extend(rest.iter().cloned());
                self.useful(rows, &q2, tys)
            }),
            Pat::Ctor {
                set, fields, span, ..
            } => set.iter().find_map(|&c| {
                let cols = self.ctor_cols(ty, c);
                let mut q2: Vec<Pat> = cols
                    .iter()
                    .map(|(n, _)| {
                        fields
                            .iter()
                            .find(|(f, _)| f == n)
                            .map(|(_, p)| p.clone())
                            .unwrap_or(Pat::wild(*span))
                    })
                    .collect();
                q2.extend(rest.iter().cloned());
                let spec = self.specialize(rows, &Head::Ctor(c), &cols);
                let w = self.useful(&spec, &q2, &with_cols(&cols))?;
                Some(rebuild(c, &cols, w))
            }),
            Pat::Lit { tm, .. } => {
                let spec = self.specialize(rows, &Head::Lit(*tm), &[]);
                let mut w = self.useful(&spec, rest, &tys[1..])?;
                w.insert(0, Witness::Any);
                Some(w)
            }
            Pat::Wild { span, .. } => {
                let mut named: Vec<u32> = Vec::new();
                for r in rows {
                    for p in r[0].alts() {
                        if let Pat::Ctor { set, .. } = p {
                            named.extend(set.iter().copied());
                        }
                    }
                }
                let sig = self.signature(ty);
                match sig {
                    Some(sig) if !sig.is_empty() && sig.iter().all(|(c, _)| named.contains(c)) => {
                        sig.iter().find_map(|&(c, _)| {
                            let cols = self.ctor_cols(ty, c);
                            let mut q2: Vec<Pat> = cols.iter().map(|_| Pat::wild(*span)).collect();
                            q2.extend(rest.iter().cloned());
                            let spec = self.specialize(rows, &Head::Ctor(c), &cols);
                            let w = self.useful(&spec, &q2, &with_cols(&cols))?;
                            Some(rebuild(c, &cols, w))
                        })
                    }
                    sig => {
                        let default: Vec<Vec<Pat>> = rows
                            .iter()
                            .flat_map(|r| {
                                r[0].alts()
                                    .into_iter()
                                    .filter(|p| matches!(p, Pat::Wild { .. }))
                                    .map(|_| r[1..].to_vec())
                                    .collect::<Vec<_>>()
                            })
                            .collect();
                        let mut w = self.useful(&default, rest, &tys[1..])?;
                        let missing = match sig {
                            Some(sig) if !named.is_empty() => sig
                                .iter()
                                .find(|(c, _)| !named.contains(c))
                                .map(|&(c, ref name)| {
                                    let fields = self
                                        .ctor_cols(ty, c)
                                        .into_iter()
                                        .map(|(n, _)| (n, Witness::Any))
                                        .collect();
                                    Witness::Ctor {
                                        name: name.clone(),
                                        fields,
                                    }
                                }),
                            _ => None,
                        };
                        w.insert(0, missing.unwrap_or(Witness::Any));
                        Some(w)
                    }
                }
            }
        }
    }

    /// Returns whether an arm with no expected type takes its type from the other arms: a numeric literal, a set literal, a lambda, or a constructor whose fields do not fix its type arguments.
    pub(super) fn deferred(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Set(_) | ExprKind::Lambda(..) => true,
            ExprKind::Ctor { .. } => !self.ctor_synthesizes(e),
            _ => expr::is_literal(e),
        }
    }

    fn ctor_synthesizes(&self, e: &Expr) -> bool {
        let ExprKind::Ctor { path, ctor, record } = &e.kind else {
            return false;
        };
        let name = &ctor.as_ref().unwrap_or(&path.name).text;
        let def = if ctor.is_some() {
            match self.lookup_type(path) {
                Ok(TypeName::Decl(d)) => d,
                _ => return true,
            }
        } else {
            match self.ctors_named(name).as_slice() {
                [(d, _)] => *d,
                _ => return true,
            }
        };
        let t = self.schema.ty(def);
        let Some(c) = t.ctor_index(name) else {
            return true;
        };
        let given: Vec<&str> = match record {
            None => Vec::new(),
            Some(ast::RecordExpr::Fields(fs, _)) => {
                fs.iter().map(|(n, _)| n.text.as_str()).collect()
            }
            Some(_) => return true,
        };
        let params: Vec<Ty> = (0..t.params.len() as u32).map(Ty::Param).collect();
        let fields = ty::ctor_fields(self.schema, def, &params, c);
        (0..params.len() as u32).all(|i| {
            fields
                .iter()
                .any(|(f, fty, _)| given.contains(&f.as_str()) && expr::mentions(fty, i))
        })
    }

    /// Elaborates an arm with no expected type: synthesized, or checked against `joined` when it takes its type from the other arms.
    pub(super) fn arm_against(&mut self, e: &Expr, joined: Option<&Ty>) -> R<Out> {
        match joined {
            Some(t) if self.deferred(e) => match (&e.kind, t) {
                (ExprKind::Ctor { .. }, Ty::Data { def, args, .. }) => {
                    let whole = Ty::Data {
                        def: *def,
                        args: args.clone(),
                        ctors: None,
                    };
                    self.check(e, &whole)
                }
                _ => {
                    let t = t.clone();
                    self.check(e, &t)
                }
            },
            _ => self.synth(e),
        }
    }

    /// Checks each arm's guard and body, against `expected` when given, else at their join; returns the match's type.
    fn arm_bodies(&mut self, arms: &[Arm], irs: &mut [ArmIr], expected: Option<&Ty>) -> R<Ty> {
        let mut joined: Option<Ty> = expected.cloned();
        let first_pass: Vec<bool> = arms
            .iter()
            .map(|a| expected.is_some() || !self.deferred(&a.body))
            .collect();
        for pass in [true, false] {
            for (i, arm) in arms.iter().enumerate() {
                if first_pass[i] != pass {
                    continue;
                }
                self.scope.push();
                for (name, local) in &irs[i].locals {
                    self.scope.bind(name, local.clone());
                }
                let r = (|| -> R<(Option<Tm>, Out)> {
                    let guard = match &arm.guard {
                        Some(g) => {
                            let bt = self.bool_ty();
                            Some(self.check(g, &bt)?.tm)
                        }
                        None => None,
                    };
                    let body = match &joined {
                        Some(t) if expected.is_some() => {
                            let t = t.clone();
                            self.check(&arm.body, &t)?
                        }
                        j => {
                            let j = j.clone();
                            self.arm_against(&arm.body, j.as_ref())?
                        }
                    };
                    Ok((guard, body))
                })();
                self.scope.pop();
                let (guard, body) = r?;
                if expected.is_none() {
                    joined = Some(match &joined {
                        None => body.ty.clone(),
                        Some(j) => self
                            .join(j, &body.ty)
                            .ok_or_else(|| self.no_join(j, &body.ty, arm.body.span))?,
                    });
                }
                irs[i].guard = guard;
                irs[i].body = Some(body);
            }
        }
        let joined = joined.expect("a match has an arm");
        for ir in irs.iter_mut() {
            let body = ir.body.take().expect("checked");
            ir.body = Some(self.widen(body, &joined));
        }
        Ok(joined)
    }

    /// The error for arms of types `a` and `b` that have no join, naming the fields missing from either.
    fn no_join(&self, a: &Ty, b: &Ty, span: Span) -> Diag {
        if let (Ty::Record(x), Ty::Record(y)) = (a, b) {
            let missing: Vec<String> = x
                .iter()
                .filter(|(n, _)| !y.iter().any(|(m, _)| m == n))
                .chain(y.iter().filter(|(n, _)| !x.iter().any(|(m, _)| m == n)))
                .map(|(n, _)| format!("field {n} missing"))
                .collect();
            if !missing.is_empty() {
                return diag(
                    span,
                    format!("the arms have no common type: {}", missing.join(", ")),
                );
            }
        }
        diag(
            span,
            format!(
                "the arms have no common type: {} and {}",
                self.show(a),
                self.show(b)
            ),
        )
    }

    /// Compiles the arms that can match constructor `c` of the scrutinee, every arm when `c` is `None`, to tests tried in order and ending in `Unreachable`.
    fn chain(&mut self, c: Option<u32>, y: Tm, y_ty: &Ty, arms: &[ArmIr], res: &Ty) -> Tm {
        let Some((first, rest)) = arms.split_first() else {
            return self.add(Kind::Unreachable, res.clone());
        };
        let alts: Vec<Pat> = first
            .pat
            .alts()
            .into_iter()
            .filter(|p| match (p, c) {
                (Pat::Ctor { set, .. }, Some(c)) => set.contains(&c),
                _ => true,
            })
            .cloned()
            .collect();
        let f = self.chain(c, y, y_ty, rest, res);
        if alts.is_empty() {
            return f;
        }
        let body = first.body.as_ref().expect("checked").tm;
        let k = match first.guard {
            Some(g) => self.add(Kind::If(g, body, f), res.clone()),
            None => body,
        };
        let cx = Cx {
            binds: &first.binds,
            res,
        };
        let mut out = f;
        for p in alts.iter().rev() {
            let cont = Cont { k, f: out };
            out = match (p, c) {
                (Pat::Ctor { .. }, Some(c)) => self.compile_fields(y, y_ty, c, p, &cx, cont),
                _ => self.compile(y, y_ty, p, &cx, cont),
            };
        }
        out
    }

    /// Matches `y` against `p`, continuing with `k`, under the binders of `p`, on a match and with `f` otherwise.
    fn compile(&mut self, y: Tm, y_ty: &Ty, p: &Pat, cx: &Cx, cont: Cont) -> Tm {
        let Cont { k, f } = cont;
        match p {
            Pat::Wild { binder: None, .. } => k,
            Pat::Wild {
                binder: Some(n), ..
            } => self.add(Kind::Let(cx.binds[n], y, k), cx.res.clone()),
            Pat::Lit { tm, .. } => {
                let bt = self.bool_ty();
                let test = self.add(Kind::Prim(Prim::Eq, vec![y, *tm]), bt);
                self.add(Kind::If(test, k, f), cx.res.clone())
            }
            Pat::Or(alts) => {
                let mut out = f;
                for a in alts.iter().rev() {
                    out = self.compile(y, y_ty, a, cx, Cont { k, f: out });
                }
                out
            }
            Pat::Ctor { set, .. } => {
                let sig = self
                    .signature(y_ty)
                    .expect("a constructor pattern is on a sum");
                let mut arms = Vec::new();
                for (d, _) in sig {
                    let row = ty::restrict(self.schema, y_ty, vec![d]);
                    let x = self.fresh(row.clone());
                    let body = if set.contains(&d) {
                        let xt = self.var_tm(x);
                        self.compile_fields(xt, &row, d, p, cx, cont)
                    } else {
                        f
                    };
                    arms.push((d, x, body));
                }
                self.add(Kind::Case { scrutinee: y, arms }, cx.res.clone())
            }
        }
    }

    /// Matches the binder and field patterns of `p` against `y`, a value known to have constructor `c`.
    fn compile_fields(&mut self, y: Tm, y_ty: &Ty, c: u32, p: &Pat, cx: &Cx, cont: Cont) -> Tm {
        let Pat::Ctor { binder, fields, .. } = p else {
            unreachable!("compile_fields takes a constructor pattern")
        };
        let cols = self.ctor_cols(y_ty, c);
        let mut out = cont.k;
        for (name, sub) in fields.iter().rev() {
            let fty = cols
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t.clone())
                .expect("a field of the constructor");
            let fy = self.add(Kind::Field(y, name.clone()), fty.clone());
            out = self.compile(fy, &fty, sub, cx, Cont { k: out, f: cont.f });
        }
        match binder {
            Some(n) => self.add(Kind::Let(cx.binds[n], y, out), cx.res.clone()),
            None => out,
        }
    }
}
