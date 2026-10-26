use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Header,
    Ctor(String),
}

impl Cx<'_> {
    /// The part of a type a field path names.
    pub(super) fn part(&self, t: &ast::TypeDecl, path: &ast::FieldPath) -> R<Part> {
        match (&t.body, &path.ctor) {
            (ast::TypeBody::New(..), _) => self.err_at(
                path.ty.span(),
                format!("newtype `{}` has no fields", t.name.text),
            ),
            (ast::TypeBody::Record(_), None) => Ok(Part::Ctor(t.name.text.clone())),
            (ast::TypeBody::Record(_), Some(c)) if c.text == t.name.text => {
                Ok(Part::Ctor(c.text.clone()))
            }
            (ast::TypeBody::Ctors(_), None) => Ok(Part::Header),
            (ast::TypeBody::Ctors(cs), Some(c)) if cs.iter().any(|d| d.name.text == c.text) => {
                Ok(Part::Ctor(c.text.clone()))
            }
            (_, Some(c)) => self.err_at(
                c.span,
                format!("`{}` is not a constructor of `{}`", c.text, t.name.text),
            ),
        }
    }

    /// The field declaration a part of a type declares under `name`.
    pub(super) fn field_decl<'t>(
        t: &'t ast::TypeDecl,
        part: &Part,
        name: &str,
    ) -> Option<&'t ast::FieldType> {
        decl_fields(t).into_iter().find_map(|(c, f)| {
            let here = match (part, c) {
                (Part::Header, None) => true,
                (Part::Ctor(p), Some(c)) => p == c,
                _ => false,
            };
            (here && f.name.text == name).then_some(f)
        })
    }

    pub(super) fn shown(t: &ast::TypeDecl, part: &Part) -> String {
        match part {
            Part::Ctor(c) if *c != t.name.text => format!("{}.{c}", t.name.text),
            _ => t.name.text.clone(),
        }
    }

    /// Refuses a field `name` the part, or anything it would clash with, already has.
    pub(super) fn field_is_free(&self, t: &ast::TypeDecl, part: &Part, name: &ast::Ident) -> R<()> {
        let clash = decl_fields(t).into_iter().any(|(c, f)| {
            f.name.text == name.text
                && match (part, c) {
                    (Part::Header, _) | (_, None) => true,
                    (Part::Ctor(p), Some(c)) => p == c,
                }
        });
        if clash {
            return self.err_at(
                name.span,
                format!(
                    "`{}` already has a field `{}`",
                    Self::shown(t, part),
                    name.text
                ),
            );
        }
        Ok(())
    }

    /// The row type a fill or routing for `part` reads: the whole sum for the header.
    pub(super) fn row_ty(schema: &Schema, def: TypeId, ctors: Option<Vec<u32>>) -> Ty {
        let t = schema.ty(def);
        Ty::Data {
            def,
            args: (0..t.params.len() as u32).map(Ty::Param).collect(),
            ctors: ctors.and_then(|cs| schema.normalize_ctors(def, cs)),
        }
    }

    pub(super) fn check_fill(
        &self,
        def_q: &QName,
        ctors: Option<Vec<u32>>,
        field_ty: &Ty,
        fill: &Fill,
    ) -> R<()> {
        let (schema, program) = self.checked()?;
        let def = self.type_id(&schema, def_q)?;
        let module = schema
            .module_named(&self.module_of(def_q))
            .unwrap_or(schema::PRELUDE);
        let row = Self::row_ty(&schema, def, ctors);
        let body = match fill {
            Fill::Match(arms) => RowBody::Match(arms, self.span),
            Fill::Expr(e) => RowBody::Expr(e),
        };
        let mut refs = Vec::new();
        check::check_row(
            &schema,
            &program,
            module,
            &row,
            body,
            Some(field_ty),
            Some(&mut refs),
        )
        .map_err(|d| vec![d])?;
        self.note(&schema, &refs);
        Ok(())
    }

    pub(super) fn add_field(
        &mut self,
        path: &ast::FieldPath,
        ty: &ast::Type,
        fill: &Fill,
    ) -> R<()> {
        let (i, t) = self.type_decl(&path.ty)?;
        let part = self.part(&t, path)?;
        self.field_is_free(&t, &part, &path.field)?;
        let item = format!("{}: {}", path.field.text, self.slice(ty.span));
        let h = header(&self.src.decls[i].module).len();
        let text = self.src.decls[i].text.clone();
        let ctor = match &part {
            Part::Header => None,
            Part::Ctor(c) => Some(c.as_str()),
        };
        let Some(edit) = add_field_edit(&text, h, &t, ctor, &item) else {
            return self.err("the field has no place in the declaration");
        };
        self.src.edit(i, vec![edit]);
        let schema = self.schema()?;
        let def = self.type_id(&schema, &path.ty)?;
        let td = schema.ty(def);
        let (ctors, fty) = match &part {
            Part::Header => (
                None,
                td.header()
                    .iter()
                    .find(|f| f.name == path.field.text)
                    .map(|f| f.ty.clone()),
            ),
            Part::Ctor(c) => {
                let k = td.ctor_index(c).expect("the constructor");
                (
                    Some(vec![k]),
                    td.field(k, &path.field.text).map(|f| f.ty.clone()),
                )
            }
        };
        let Some(fty) = fty else {
            return self.err("the added field does not resolve");
        };
        self.check_fill(&path.ty, ctors, &fty, fill)
    }

    /// The edit that removes field `name` from a part of declaration `i`.
    pub(super) fn field_removal(
        &self,
        i: usize,
        t: &ast::TypeDecl,
        part: &Part,
        name: &str,
    ) -> Option<(Range<usize>, String)> {
        let f = Self::field_decl(t, part, name)?;
        let h = header(&self.src.decls[i].module).len();
        let text = &self.src.decls[i].text;
        let ctor = match part {
            Part::Header => None,
            Part::Ctor(c) => Some(c.as_str()),
        };
        let braces = field_braces(text, h, t, ctor)?;
        let (range, _) = remove_item(text, braces, f.name.span.start as usize - h);
        Some((range, String::new()))
    }

    pub(super) fn drop_field(&mut self, path: &ast::FieldPath) -> R<()> {
        let (i, t) = self.type_decl(&path.ty)?;
        let part = self.part(&t, path)?;
        let Some(edit) = self.field_removal(i, &t, &part, &path.field.text) else {
            return self.err_at(
                path.field.span,
                format!(
                    "`{}` has no field `{}`",
                    Self::shown(&t, &part),
                    path.field.text
                ),
            );
        };
        self.src.edit(i, vec![edit]);
        Ok(())
    }

    pub(super) fn rename_field(&mut self, path: &ast::FieldPath, to: &ast::Ident) -> R<()> {
        let (i, t) = self.type_decl(&path.ty)?;
        let part = self.part(&t, path)?;
        let from = path.field.text.clone();
        let Some(f) = Self::field_decl(&t, &part, &from) else {
            return self.err_at(
                path.field.span,
                format!("`{}` has no field `{from}`", Self::shown(&t, &part)),
            );
        };
        self.field_is_free(&t, &part, to)?;
        let schema = self.schema()?;
        let def = self.type_id(&schema, &path.ty)?;
        let ctor = match &part {
            Part::Header => None,
            Part::Ctor(c) => schema.ty(def).ctor_index(c),
        };
        let mut edits: Edits = BTreeMap::new();
        edits
            .entry(i)
            .or_default()
            .push((self.src.local(i, f.name.span), to.text.clone()));
        for (file, span, r) in check::schema_refs(&schema) {
            if let Ref::Field {
                def: d,
                ctor: c,
                name,
                pun,
            } = r
                && d == def
                && c == ctor
                && name == from
            {
                let j = file.0 as usize;
                let text = if pun {
                    format!("{}: {from}", to.text)
                } else {
                    to.text.clone()
                };
                edits
                    .entry(j)
                    .or_default()
                    .push((self.src.local(j, span), text));
            }
        }
        let cname = match &part {
            Part::Ctor(c) => Some(c.clone()),
            Part::Header => None,
        };
        for (_, m, rname) in Self::relations_of(&schema, def) {
            let Some(k) = self.src.find("relation", &m, &rname) else {
                continue;
            };
            let Some(Decl::Relation(rd)) = self.src.parsed(k) else {
                continue;
            };
            for cl in &rd.clauses {
                match cl {
                    RelationClause::Key(fs, _) => {
                        for x in fs {
                            if x.text == from {
                                edits
                                    .entry(k)
                                    .or_default()
                                    .push((self.src.local(k, x.span), to.text.clone()));
                            }
                        }
                    }
                    RelationClause::Default {
                        ctor: dc, field, ..
                    } if field.text == from
                        && (cname.is_none()
                            || dc.is_none()
                            || dc.as_ref().map(|x| &x.text) == cname.as_ref()) =>
                    {
                        edits
                            .entry(k)
                            .or_default()
                            .push((self.src.local(k, field.span), to.text.clone()));
                    }
                    _ => {}
                }
            }
        }
        self.apply_edits(edits);
        Ok(())
    }

    pub(super) fn move_field(
        &mut self,
        path: &ast::FieldPath,
        to: &QName,
        sel: Option<&CtorSel>,
        fill: Option<&Fill>,
    ) -> R<()> {
        let (i, t) = self.type_decl(&path.ty)?;
        if to.name.text != path.ty.name.text || self.module_of(to) != self.module_of(&path.ty) {
            return self.err_at(to.span(), "a field moves within its own type");
        }
        let part = self.part(&t, path)?;
        let Some(f) = Self::field_decl(&t, &part, &path.field.text) else {
            return self.err_at(
                path.field.span,
                format!(
                    "`{}` has no field `{}`",
                    Self::shown(&t, &part),
                    path.field.text
                ),
            );
        };
        let h = header(&self.src.decls[i].module).len();
        let text = self.src.decls[i].text.clone();
        let fty_text = text[f.ty.span.start as usize - h..f.ty.span.end as usize - h].to_string();
        let item = format!("{}: {fty_text}", path.field.text);
        let ast::TypeBody::Ctors(cs) = &t.body else {
            return self.err_at(
                path.ty.span(),
                format!(
                    "record `{}` has no header to move a field to or from",
                    t.name.text
                ),
            );
        };
        match (&part, sel) {
            (Part::Ctor(c), None) => {
                let others: Vec<String> = cs
                    .iter()
                    .map(|d| d.name.text.clone())
                    .filter(|d| d != c)
                    .collect();
                for o in &others {
                    if Self::field_decl(&t, &Part::Ctor(o.clone()), &path.field.text).is_some() {
                        return self.err(format!(
                            "`{}.{o}` already has a field `{}`; the header cannot take it",
                            t.name.text, path.field.text
                        ));
                    }
                }
                let removal = self
                    .field_removal(i, &t, &part, &path.field.text)
                    .expect("the field");
                let added = add_field_edit(&text, h, &t, None, &item).expect("a header place");
                self.src.edit(i, vec![removal, added]);
                if others.is_empty() {
                    return Ok(());
                }
                let Some(fill) = fill else {
                    return self.err(
                        "a move to the header takes `fill match` with an arm for each constructor that lacks the field",
                    );
                };
                if matches!(fill, Fill::Expr(_)) {
                    return self.err(
                        "the fill of a move to the header is a `match` whose arms are the constructors that lack the field",
                    );
                }
                let schema = self.schema()?;
                let def = self.type_id(&schema, &path.ty)?;
                let td = schema.ty(def);
                let ctors: Vec<u32> = others.iter().filter_map(|o| td.ctor_index(o)).collect();
                let fty = td
                    .header()
                    .iter()
                    .find(|x| x.name == path.field.text)
                    .map(|x| x.ty.clone());
                let Some(fty) = fty else {
                    return self.err("the moved field does not resolve");
                };
                self.check_fill(&path.ty, Some(ctors), &fty, fill)
            }
            (Part::Header, Some(sel)) => {
                if fill.is_some() {
                    return self.err("a move from the header down to constructors takes no fill");
                }
                let mut targets = Vec::new();
                for x in sel_idents(sel) {
                    if !cs.iter().any(|d| d.name.text == x.text) {
                        return self.err_at(
                            x.span,
                            format!("`{}` is not a constructor of `{}`", x.text, t.name.text),
                        );
                    }
                    targets.push(x.text.clone());
                }
                let removal = self
                    .field_removal(i, &t, &part, &path.field.text)
                    .expect("the field");
                let mut edits = vec![removal];
                for c in &targets {
                    edits.push(
                        add_field_edit(&text, h, &t, Some(c), &item).expect("a constructor place"),
                    );
                }
                self.src.edit(i, edits);
                Ok(())
            }
            (Part::Ctor(_), Some(_)) => self.err(
                "a constructor's field moves to the header: `move field T.C.f to T fill match ...`",
            ),
            (Part::Header, None) => {
                self.err("a header field moves down to constructors: `move field T.f to T.(C | D)`")
            }
        }
    }
}

impl Cx<'_> {
    /// The text of the expressions `fields`, as written in the migration.
    pub(super) fn fields_text(&self, fields: &[Expr]) -> String {
        let first = fields.first().expect("an index has a field").span;
        let last = fields.last().expect("an index has a field").span;
        self.text[first.start as usize..last.end as usize].to_string()
    }

    pub(super) fn scope_text(&self, on: &ast::RelationRef) -> String {
        match &on.sel {
            None => String::new(),
            Some(sel) => {
                let names: Vec<String> = sel_idents(sel).iter().map(|x| x.text.clone()).collect();
                format!(" in {}", sel_text(&names))
            }
        }
    }

    /// Appends a clause to the declaration of relation `q`.
    pub(super) fn append_clause(&mut self, q: &QName, clause: &str) -> R<()> {
        let (i, _) = self.relation_decl(q)?;
        let end = self.src.decls[i].text.trim_end().len();
        self.src.edit(i, vec![(end..end, format!("\n  {clause}"))]);
        Ok(())
    }

    pub(super) fn add_index(
        &mut self,
        kind: &str,
        fields: &[Expr],
        on: &ast::RelationRef,
    ) -> R<()> {
        let clause = format!(
            "{kind} ({}){}",
            self.fields_text(fields),
            self.scope_text(on)
        );
        self.append_clause(&on.relation, &clause)
    }

    /// The clauses of relation `q`'s declaration with their ranges.
    pub(super) fn clauses(&self, q: &QName) -> R<(usize, ast::RelationDecl, Vec<Range<usize>>)> {
        let (i, rd) = self.relation_decl(q)?;
        let h = header(&self.src.decls[i].module).len();
        match clause_ranges(&self.src.decls[i].text, h, &rd) {
            Some(rs) => Ok((i, rd, rs)),
            None => self.err_at(
                q.span(),
                "the clauses of the relation declaration cannot be told apart",
            ),
        }
    }

    pub(super) fn drop_clause(&mut self, i: usize, ranges: &[Range<usize>], k: usize) {
        let text = self.src.decls[i].text.clone();
        self.src
            .edit(i, vec![(clause_removal(&text, ranges, k), String::new())]);
    }

    pub(super) fn drop_index(
        &mut self,
        unique: bool,
        interval: bool,
        fields: &[Expr],
        on: &ast::RelationRef,
    ) -> R<()> {
        let schema = self.schema()?;
        let rel = self.rel_id(&schema, &on.relation)?;
        let (i, rd, ranges) = self.clauses(&on.relation)?;
        let all = |s: Option<CtorSet>| {
            s.and_then(|cs| schema.normalize_ctors(schema.relation(rel).elem_def, cs))
        };
        let want_scope = match &on.sel {
            None => None,
            Some(sel) => all(Some(
                schema
                    .relation_sel(rel, sel)
                    .map_err(|e| vec![Diag::new(on.span, e)])?,
            )),
        };
        let want: Vec<String> = fields
            .iter()
            .map(|e| self.slice(e.span).to_string())
            .collect();
        let h = header(&self.src.decls[i].module).len();
        let text = self.src.decls[i].text.clone();
        let field_texts = |fs: &[Expr]| -> Vec<String> {
            fs.iter()
                .map(|e| text[e.span.start as usize - h..e.span.end as usize - h].to_string())
                .collect()
        };
        let same = |a: &[String], b: &[String]| {
            a.len() == b.len()
                && a.iter().zip(b).all(|(x, y)| {
                    lex(x)
                        .0
                        .iter()
                        .map(|t| &x[t.span.range()])
                        .collect::<Vec<_>>()
                        == lex(y)
                            .0
                            .iter()
                            .map(|t| &y[t.span.range()])
                            .collect::<Vec<_>>()
                })
        };
        for (k, cl) in rd.clauses.iter().enumerate() {
            let (fs, scope) = match cl {
                RelationClause::Unique(fs, s) if unique => (fs, s),
                RelationClause::Index {
                    interval: iv,
                    fields: fs,
                    scope: s,
                } if !unique && *iv == interval => (fs, s),
                _ => continue,
            };
            let scope = match scope {
                None => None,
                Some(s) => all(schema.relation_sel(rel, &scope_sel(s)).ok()),
            };
            if scope == want_scope && same(&field_texts(fs), &want) {
                self.drop_clause(i, &ranges, k);
                return Ok(());
            }
        }
        let kind = if unique {
            "unique"
        } else if interval {
            "interval index"
        } else {
            "index"
        };
        self.err(format!(
            "`{}` has no {kind} ({}){}",
            rd.name.name.text,
            want.join(", "),
            self.scope_text(on)
        ))
    }

    pub(super) fn add_default(
        &mut self,
        relation: &QName,
        ctor: Option<&ast::Ident>,
        field: &ast::Ident,
        value: &ast::DefaultValue,
    ) -> R<()> {
        let path = match ctor {
            Some(c) => format!("{}.{}", c.text, field.text),
            None => field.text.clone(),
        };
        let v = match value {
            ast::DefaultValue::Generate(g) => format!("generate {}", g.text),
            ast::DefaultValue::Expr(e) => self.slice(e.span).to_string(),
        };
        self.append_clause(relation, &format!("default {path} = {v}"))
    }

    pub(super) fn drop_default(
        &mut self,
        relation: &QName,
        ctor: Option<&ast::Ident>,
        field: &ast::Ident,
    ) -> R<()> {
        let (i, rd, ranges) = self.clauses(relation)?;
        for (k, cl) in rd.clauses.iter().enumerate() {
            if let RelationClause::Default {
                ctor: c, field: f, ..
            } = cl
                && f.text == field.text
                && c.as_ref().map(|x| &x.text) == ctor.map(|x| &x.text)
            {
                self.drop_clause(i, &ranges, k);
                return Ok(());
            }
        }
        self.err_at(
            field.span,
            format!(
                "`{}` has no default for `{}`",
                relation.name.text, field.text
            ),
        )
    }

    pub(super) fn computed_clause(
        &self,
        relation: &QName,
        field: &ast::Ident,
    ) -> R<(usize, Vec<Range<usize>>, usize)> {
        let (i, rd, ranges) = self.clauses(relation)?;
        for (k, cl) in rd.clauses.iter().enumerate() {
            if let RelationClause::Computed(n, _, _) = cl
                && n.text == field.text
            {
                return Ok((i, ranges, k));
            }
        }
        self.err_at(
            field.span,
            format!(
                "`{}` has no computed field `{}`",
                relation.name.text, field.text
            ),
        )
    }

    pub(super) fn add_computed(
        &mut self,
        relation: &QName,
        field: &ast::Ident,
        ty: &ast::Type,
        expr: &Expr,
    ) -> R<()> {
        let clause = format!(
            "computed {}: {} = {}",
            field.text,
            self.slice(ty.span),
            self.slice(expr.span)
        );
        self.append_clause(relation, &clause)
    }

    pub(super) fn redefine_computed(
        &mut self,
        relation: &QName,
        field: &ast::Ident,
        ty: &ast::Type,
        expr: &Expr,
    ) -> R<()> {
        let (i, ranges, k) = self.computed_clause(relation, field)?;
        let clause = format!(
            "computed {}: {} = {}",
            field.text,
            self.slice(ty.span),
            self.slice(expr.span)
        );
        self.src.edit(i, vec![(ranges[k].clone(), clause)]);
        Ok(())
    }

    pub(super) fn drop_computed(&mut self, relation: &QName, field: &ast::Ident) -> R<()> {
        let (i, ranges, k) = self.computed_clause(relation, field)?;
        self.drop_clause(i, &ranges, k);
        Ok(())
    }

    pub(super) fn rekey(
        &mut self,
        relation: &QName,
        key: &[ast::Ident],
        carry: &[ast::FieldPath],
    ) -> R<()> {
        let schema = self.schema()?;
        let rel = self.rel_id(&schema, relation)?;
        let own = schema.relation(rel).elem_def;
        let rname = schema.relation(rel).name.clone();
        let mut needed: Vec<(TypeId, Option<String>, String)> = Vec::new();
        for (j, t) in schema.types.iter().enumerate() {
            let def = TypeId(j as u32);
            if def == own || schema.module(t.module).engine {
                continue;
            }
            for f in t.header() {
                if refers(&f.ty, rel) {
                    needed.push((def, None, f.name.clone()));
                }
            }
            for c in t.ctors() {
                for f in &c.fields {
                    if refers(&f.ty, rel) {
                        let ctor =
                            (t.ctors().len() > 1 || c.name != t.name).then(|| c.name.clone());
                        needed.push((def, ctor, f.name.clone()));
                    }
                }
            }
        }
        let mut listed = Vec::new();
        let mut errs = Vec::new();
        for p in carry {
            let def = match self.type_id(&schema, &p.ty) {
                Ok(d) => d,
                Err(mut e) => {
                    errs.append(&mut e);
                    continue;
                }
            };
            let entry = (
                def,
                p.ctor.as_ref().map(|c| c.text.clone()),
                p.field.text.clone(),
            );
            if !needed.contains(&entry) {
                errs.push(Diag::new(
                    p.field.span,
                    format!(
                        "`{}` is not a field that references `{rname}`",
                        path_text(p)
                    ),
                ));
            }
            listed.push(entry);
        }
        for (def, ctor, f) in &needed {
            if !listed.contains(&(*def, ctor.clone(), f.clone())) {
                let t = &schema.ty(*def).name;
                let p = match ctor {
                    Some(c) => format!("{t}.{c}.{f}"),
                    None => format!("{t}.{f}"),
                };
                errs.push(Diag::new(
                    self.span,
                    format!("`rekey relation {rname}` is refused: field `{p}` references `{rname}` and is not carried"),
                ));
            }
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        let (i, rd, ranges) = self.clauses(relation)?;
        let Some(k) = rd
            .clauses
            .iter()
            .position(|c| matches!(c, RelationClause::Key(..)))
        else {
            return self.err(format!("`{rname}` declares no key"));
        };
        let names: Vec<&str> = key.iter().map(|x| x.text.as_str()).collect();
        let clause = match names.as_slice() {
            [one] => format!("key {one}"),
            many => format!("key ({})", many.join(", ")),
        };
        self.src.edit(i, vec![(ranges[k].clone(), clause)]);
        Ok(())
    }
}

pub(super) fn refers(t: &Ty, rel: RelId) -> bool {
    match t {
        Ty::Key { rel: r, .. } => *r == rel,
        Ty::Data { args, .. } => args.iter().any(|a| refers(a, rel)),
        _ => false,
    }
}

pub(super) fn path_text(p: &ast::FieldPath) -> String {
    match &p.ctor {
        Some(c) => format!("{}.{}.{}", p.ty.name.text, c.text, p.field.text),
        None => format!("{}.{}", p.ty.name.text, p.field.text),
    }
}
