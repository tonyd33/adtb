use super::write::Site;
use super::*;
use crate::ast::{Expr, RecordExpr};
use crate::kernel::Kind;
use crate::schema::TypeDef;

impl Checker<'_> {
    pub(super) fn record(
        &mut self,
        r: &RecordExpr,
        expected: Option<&Ty>,
        span: Span,
        site: Option<&Site>,
    ) -> R<Out> {
        if let Some(t @ Ty::Data { def, ctors, .. }) = expected
            && matches!(self.schema.ty(*def).kind, TypeDefKind::Sum { .. })
        {
            let single = match ctors {
                Some(cs) if cs.len() == 1 => Some(cs[0]),
                None if self.schema.ty(*def).ctors().len() == 1 => Some(0),
                _ => None,
            };
            if let Some(c) = single {
                let Ty::Data { def, args, .. } = t else {
                    unreachable!()
                };
                return self.build_ctor(
                    Target::Decl(*def, Some(args.clone())),
                    c,
                    Some(r),
                    span,
                    site,
                );
            }
        }
        match r {
            RecordExpr::Fields(fs, _) => {
                let want: Option<Vec<(String, Ty)>> = match expected {
                    Some(Ty::Record(ws)) => Some(ws.clone()),
                    _ => None,
                };
                let mut out: Vec<(String, Tm)> = Vec::new();
                let mut tys: Vec<(String, Ty)> = Vec::new();
                for (name, e) in fs {
                    if out.iter().any(|(n, _)| *n == name.text) {
                        return Err(diag(
                            name.span,
                            format!("field `{}` is given twice", name.text),
                        ));
                    }
                    let o = match want
                        .as_ref()
                        .and_then(|ws| ws.iter().find(|(n, _)| *n == name.text))
                    {
                        Some((_, t)) => {
                            let o = self.check(e, t)?;
                            Out { ty: t.clone(), ..o }
                        }
                        None => self.synth(e)?,
                    };
                    out.push((name.text.clone(), o.tm));
                    tys.push((name.text.clone(), o.ty));
                }
                out.sort_by(|a, b| a.0.cmp(&b.0));
                tys.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(self.out(Kind::Record(out), Ty::Record(tys)))
            }
            RecordExpr::With(base, fs, _) => {
                let b = self.synth(base)?;
                let Ty::Record(bfs) = &b.ty else {
                    return Err(diag(
                        span,
                        format!(
                            "`{{ x with ... }}` copies an anonymous record; for {} name the constructor: `C {{ x with ... }}`",
                            self.show(&b.ty)
                        ),
                    ));
                };
                let mut out = Vec::new();
                let mut tys = Vec::new();
                for (name, fty) in bfs.clone() {
                    match fs.iter().find(|(n, _)| n.text == name) {
                        Some((_, e)) => {
                            let o = self.check(e, &fty)?;
                            out.push((name.clone(), o.tm));
                        }
                        None => {
                            let ft = self.terms.transport_field(b.tm, name.clone(), fty.clone());
                            out.push((name.clone(), ft));
                        }
                    }
                    tys.push((name, fty));
                }
                if let Some((n, _)) = fs
                    .iter()
                    .find(|(n, _)| !bfs.iter().any(|(f, _)| *f == n.text))
                {
                    return Err(diag(
                        n.span,
                        format!("the record has no field `{}`", n.text),
                    ));
                }
                Ok(self.out(Kind::Record(out), Ty::Record(tys)))
            }
            RecordExpr::Elided(s) => Err(diag(*s, "an elided record cannot be checked")),
        }
    }

    /// Resolves a constructor expression against the expected type, else by its name in scope, then builds it.
    pub(super) fn ctor(
        &mut self,
        path: &ast::QName,
        ctor: Option<&ast::Ident>,
        record: Option<&RecordExpr>,
        expected: Option<&Ty>,
        span: Span,
        site: Option<&Site>,
    ) -> R<Out> {
        let name = match ctor {
            Some(c) => &c.text,
            None => &path.name.text,
        };
        let known_args = |def: TypeId| match expected {
            Some(Ty::Data { def: d, args, .. }) if *d == def => Some(args.clone()),
            _ => None,
        };
        let (target, c) = if ctor.is_some() {
            let def = match self.lookup_type(path) {
                Ok(TypeName::Decl(d)) => d,
                Ok(_) => {
                    return Err(diag(
                        path.span(),
                        format!("`{}` has no constructors", path.name.text),
                    ));
                }
                Err(e) => return Err(diag(path.span(), e)),
            };
            self.note(path.name.span, Ref::Type(def));
            let c = self.schema.ty(def).ctor_index(name).ok_or_else(|| {
                diag(
                    span,
                    format!(
                        "`{name}` is not a constructor of `{}`",
                        self.schema.ty(def).name
                    ),
                )
            })?;
            (Target::Decl(def, known_args(def)), c)
        } else {
            let from_expected = match expected {
                Some(Ty::Data { def, args, .. }) => self
                    .schema
                    .ty(*def)
                    .ctor_index(name)
                    .map(|c| (Target::Decl(*def, Some(args.clone())), c)),
                Some(Ty::Sum { ctors, sel }) => ctors
                    .iter()
                    .position(|(c, _)| c == name)
                    .filter(|i| sel.as_ref().is_none_or(|s| s.contains(&(*i as u32))))
                    .map(|i| {
                        let row = Ty::Sum {
                            ctors: ctors.clone(),
                            sel: (ctors.len() != 1).then(|| vec![i as u32]),
                        };
                        (Target::Known(row), i as u32)
                    }),
                Some(t @ Ty::Tag(inner)) => match inner.as_ref() {
                    Ty::Data { def, .. } => self
                        .schema
                        .ty(*def)
                        .ctor_index(name)
                        .map(|c| (Target::Known(t.clone()), c)),
                    _ => None,
                },
                _ => None,
            };
            match from_expected {
                Some(r) if path.module.is_none() => r,
                _ => {
                    let found: Vec<(TypeId, u32)> = match &path.module {
                        Some(_) => match self.lookup_type(path) {
                            Ok(TypeName::Decl(d)) => self
                                .schema
                                .ty(d)
                                .ctor_index(name)
                                .map(|c| vec![(d, c)])
                                .unwrap_or_default(),
                            _ => Vec::new(),
                        },
                        None => self.ctors_named(name),
                    };
                    match found.as_slice() {
                        [(def, c)] => (Target::Decl(*def, known_args(*def)), *c),
                        [] if self.adhoc() && path.module.is_none() => {
                            self.note(path.name.span, Ref::Anonymous);
                            return self.anon_ctor(name, record, span);
                        }
                        [] => {
                            return Err(diag(
                                path.span(),
                                match expected {
                                    Some(t) => {
                                        format!("`{name}` is not a constructor of {}", self.show(t))
                                    }
                                    None => format!(
                                        "no constructor `{name}` in scope, and no expected type declares it"
                                    ),
                                },
                            ));
                        }
                        many => {
                            return Err(diag(
                                path.span(),
                                format!(
                                    "`{name}` is a constructor of {}; qualify it as `T.{name}`",
                                    many.iter()
                                        .map(|(d, _)| format!("`{}`", self.schema.ty(*d).name))
                                        .collect::<Vec<_>>()
                                        .join(" and ")
                                ),
                            ));
                        }
                    }
                }
            }
        };
        let at = ctor.map_or(path.name.span, |c| c.span);
        if let Target::Known(_) = &target {
            self.note(at, Ref::Anonymous);
        }
        if let Target::Decl(def, _) = &target {
            self.note(at, Ref::Ctor(*def, c));
            let t = self.schema.ty(*def);
            if *def == self.schema.prelude_type("Span") {
                return Err(diag(span, "a span is built only by `span(start, end)`"));
            }
            if matches!(t.kind, TypeDefKind::Newtype { .. }) {
                return Err(diag(
                    span,
                    format!("newtype `{}` is built by a call: `{}(v)`", t.name, t.name),
                ));
            }
        }
        self.build_ctor(target, c, record, span, site)
    }

    fn anon_ctor(&mut self, name: &str, record: Option<&RecordExpr>, span: Span) -> R<Out> {
        let fields = match record {
            None => Vec::new(),
            Some(RecordExpr::Fields(fs, _)) => fs.clone(),
            Some(_) => {
                return Err(diag(
                    span,
                    format!("`{name} {{ x with ... }}` needs a declared constructor"),
                ));
            }
        };
        let mut tms = Vec::new();
        let mut tys = Vec::new();
        for (n, e) in &fields {
            let o = self.synth(e)?;
            tms.push((n.text.clone(), o.tm));
            tys.push((n.text.clone(), o.ty));
        }
        tms.sort_by(|a, b| a.0.cmp(&b.0));
        tys.sort_by(|a, b| a.0.cmp(&b.0));
        let ty = Ty::Sum {
            ctors: vec![(name.to_string(), tys)],
            sel: None,
        };
        Ok(self.out(
            Kind::Ctor {
                ctor: 0,
                fields: tms,
            },
            ty,
        ))
    }

    /// Builds constructor `c` of `target` from `record`, giving every field.
    pub(super) fn build_ctor(
        &mut self,
        target: Target,
        c: u32,
        record: Option<&RecordExpr>,
        span: Span,
        site: Option<&Site>,
    ) -> R<Out> {
        let (def, args) = match target {
            Target::Known(ty) => {
                let fields: Vec<(String, Ty)> = ty::fields_of_ctor(self.schema, &ty, c)
                    .into_iter()
                    .map(|(n, t, _)| (n, t))
                    .collect();
                let given = self.record_fields(&fields, record, span, &ty, c)?;
                return Ok(self.out(
                    Kind::Ctor {
                        ctor: c,
                        fields: given,
                    },
                    ty,
                ));
            }
            Target::Decl(def, args) => (def, args),
        };
        let site = site.filter(|s| self.schema.relation(s.rel).elem_def == def);
        let t = self.schema.ty(def).clone();
        let raw: Vec<(String, Ty, bool)> = ty::ctor_fields(
            self.schema,
            def,
            &(0..t.params.len() as u32)
                .map(Ty::Param)
                .collect::<Vec<_>>(),
            c,
        );
        let shape = CtorShape {
            def,
            c,
            ctor_name: t.ctors()[c as usize].name.clone(),
            t,
            raw,
            span,
            site,
        };
        let (named, base) = self.given_fields(&shape, record)?;
        let (mut given, args_out) = self.ctor_args(&shape, &named, args, base.as_ref())?;
        self.fill_elided(&shape, base.as_ref(), &args_out, &mut given)?;
        given.sort_by(|a, b| a.0.cmp(&b.0));
        let ty = Ty::Data {
            def,
            args: args_out,
            ctors: self.schema.normalize_ctors(def, vec![c]),
        };
        Ok(self.out(
            Kind::Ctor {
                ctor: c,
                fields: given,
            },
            ty,
        ))
    }

    /// Returns the fields `record` gives, by name, and the value a `with` record copies from.
    fn given_fields<'e>(
        &mut self,
        shape: &CtorShape,
        record: Option<&'e RecordExpr>,
    ) -> R<(Named<'e>, Option<Out>)> {
        let ctor_name = &shape.ctor_name;
        let mut named: Named = Vec::new();
        let mut base: Option<Out> = None;
        match record {
            None => {
                if let Some((f, _, _)) = shape.raw.first() {
                    return Err(diag(
                        shape.span,
                        format!(
                            "`{ctor_name}` has fields, starting with `{f}`; give them: `{ctor_name} {{ ... }}`"
                        ),
                    ));
                }
            }
            Some(RecordExpr::Elided(s)) => {
                return Err(diag(*s, "an elided record cannot be checked"));
            }
            Some(RecordExpr::Fields(fs, _) | RecordExpr::With(_, fs, _)) => {
                for (name, e) in fs {
                    if named.iter().any(|(n, _)| *n == name.text) {
                        return Err(diag(
                            name.span,
                            format!("field `{}` is given twice", name.text),
                        ));
                    }
                    if !shape.raw.iter().any(|(n, _, _)| *n == name.text) {
                        return Err(diag(
                            name.span,
                            format!("`{ctor_name}` has no field `{}`", name.text),
                        ));
                    }
                    self.note_field(
                        name.span,
                        shape.def,
                        Some(&[shape.c]),
                        &name.text,
                        e.span == name.span,
                    );
                    named.push((name.text.clone(), e));
                }
                if let Some(s) = shape.site {
                    self.site_given(s, fs)?;
                }
                if let Some(RecordExpr::With(b, _, _)) = record {
                    let o = self.synth(b)?;
                    if !matches!(&o.ty, Ty::Data { def: d, .. } if *d == shape.def) {
                        return Err(diag(
                            b.span,
                            format!(
                                "`{ctor_name} {{ x with ... }}` copies from a value of `{}`, found {}",
                                shape.t.name,
                                self.show(&o.ty)
                            ),
                        ));
                    }
                    base = Some(o);
                }
            }
        }
        Ok((named, base))
    }

    /// Returns the terms of the `named` fields and the constructor's type arguments, taken from
    /// `args`, else from `base`, else solved from the fields.
    fn ctor_args(
        &mut self,
        shape: &CtorShape,
        named: &[(String, &Expr)],
        args: Option<Vec<Ty>>,
        base: Option<&Out>,
    ) -> R<(Given, Vec<Ty>)> {
        let t = &shape.t;
        let args = args.or_else(|| match base {
            Some(Out {
                ty: Ty::Data { args, .. },
                ..
            }) => Some(args.clone()),
            _ => None,
        });
        let (names, (params, es)): (Vec<String>, (Vec<Ty>, Vec<&Expr>)) = shape
            .raw
            .iter()
            .filter_map(|(f, fty, _)| {
                named
                    .iter()
                    .find(|(n, _)| n == f)
                    .map(|(_, e)| (f.clone(), (fty.clone(), *e)))
            })
            .unzip();
        let (tms, sol) = match &args {
            Some(a) => {
                let params: Vec<Ty> = params.iter().map(|p| p.subst(a)).collect();
                let (tms, _) = self.solve_args(&params, &es, 0, None)?;
                (tms, a.iter().cloned().map(Some).collect())
            }
            None => self.solve_args(&params, &es, t.params.len(), None)?,
        };
        let mut args_out = Vec::new();
        for (i, a) in sol.into_iter().enumerate() {
            match a {
                Some(a) => args_out.push(a),
                None => {
                    return Err(diag(
                        shape.span,
                        format!(
                            "the type argument `{}` of `{}` is not known here; give the expected type",
                            t.params[i], t.name
                        ),
                    ));
                }
            }
        }
        Ok((names.into_iter().zip(tms).collect(), args_out))
    }

    /// Give every field `given` lacks, from `base`, the carried row or the site's defaults.
    fn fill_elided(
        &mut self,
        shape: &CtorShape,
        base: Option<&Out>,
        args: &[Ty],
        given: &mut Given,
    ) -> R<()> {
        let c = shape.c;
        let ctor_name = &shape.ctor_name;
        let same_ctor = base.is_some_and(|b| match &b.ty {
            Ty::Data { ctors: Some(s), .. } => *s == [c],
            Ty::Data { ctors: None, .. } => shape.t.ctors().len() == 1,
            _ => false,
        });
        for (f, fty, header) in &shape.raw {
            if given.iter().any(|(g, _)| g == f) {
                continue;
            }
            let fty = fty.subst(args);
            match base {
                Some(b) if same_ctor || *header => {
                    let ft = self.terms.transport_field(b.tm, f.clone(), fty);
                    given.push((f.clone(), ft));
                }
                None if *header && self.carry.is_some_and(|(_, d)| d == shape.def) => {
                    let (row, _) = self.carry.expect("a carried row");
                    let rt = self.var_tm(row);
                    let ft = self.terms.transport_field(rt, f.clone(), fty);
                    given.push((f.clone(), ft));
                }
                _ => match shape.site {
                    Some(s) => match self.fill(s, c, f, *header, fty)? {
                        Some(ft) => given.push((f.clone(), ft)),
                        None => {
                            return Err(diag(
                                shape.span,
                                format!(
                                    "field `{f}` of `{ctor_name}` is missing, and `{}` has no default for it",
                                    self.schema.relation(s.rel).name
                                ),
                            ));
                        }
                    },
                    None => {
                        return Err(diag(
                            shape.span,
                            format!("field `{f}` of `{ctor_name}` is missing"),
                        ));
                    }
                },
            }
        }
        Ok(())
    }

    /// The fields of a constructor of an anonymous sum or tag, each checked against its type.
    fn record_fields(
        &mut self,
        fields: &[(String, Ty)],
        record: Option<&RecordExpr>,
        span: Span,
        ty: &Ty,
        c: u32,
    ) -> R<Vec<(String, Tm)>> {
        let name = ty::ctor_names(self.schema, ty)
            .and_then(|ns| ns.into_iter().find(|(i, _)| *i == c).map(|(_, n)| n))
            .unwrap_or_default();
        let fs = match record {
            None => Vec::new(),
            Some(RecordExpr::Fields(fs, _)) => fs.clone(),
            Some(RecordExpr::With(..)) => {
                return Err(diag(
                    span,
                    format!("`{name} {{ x with ... }}` needs a declared constructor"),
                ));
            }
            Some(RecordExpr::Elided(s)) => {
                return Err(diag(*s, "an elided record cannot be checked"));
            }
        };
        let mut out = Vec::new();
        for (n, e) in &fs {
            let Some((_, fty)) = fields.iter().find(|(f, _)| *f == n.text) else {
                return Err(diag(n.span, format!("`{name}` has no field `{}`", n.text)));
            };
            out.push((n.text.clone(), self.check(e, fty)?.tm));
        }
        if let Some((f, _)) = fields
            .iter()
            .find(|(f, _)| !out.iter().any(|(g, _)| g == f))
        {
            return Err(diag(span, format!("field `{f}` of `{name}` is missing")));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

/// Where a constructor expression resolved: a declared type with its arguments when known, or an anonymous sum or tag type.
pub(super) enum Target {
    Decl(TypeId, Option<Vec<Ty>>),
    Known(Ty),
}

type Named<'e> = Vec<(String, &'e Expr)>;

type Given = Vec<(String, Tm)>;

/// A declared constructor being built, with its fields over the type's own parameters.
struct CtorShape<'s> {
    def: TypeId,
    c: u32,
    t: TypeDef,
    ctor_name: String,
    raw: Vec<(String, Ty, bool)>,
    span: Span,
    site: Option<&'s Site>,
}
