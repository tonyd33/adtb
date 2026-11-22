use super::*;
use crate::ast;
use crate::evolve::SourceDecl;
use crate::parse::{Options, parse_schema};
use crate::schema;
use crate::span::{FileDiag, FileId, Span};

/// A declaration being printed, with the range each term of it takes.
#[derive(Default)]
pub(super) struct Printed {
    text: String,
    /// The range of the text each term takes, in the order [`Model::decl_terms_mut`] gives
    /// the terms.
    terms: Vec<std::ops::Range<usize>>,
    /// The range of the text each arm held as written takes, by its key.
    arms: Vec<(u64, std::ops::Range<usize>)>,
}

impl Printed {
    /// Returns the range of the text each term takes, in the order [`Model::decl_terms_mut`]
    /// gives the terms.
    pub(super) fn terms(&self) -> &[std::ops::Range<usize>] {
        &self.terms
    }

    /// Returns the range of the text each arm held as written takes, by its key.
    pub(super) fn arms(&self) -> &[(u64, std::ops::Range<usize>)] {
        &self.arms
    }

    fn push(&mut self, s: &str) {
        self.text.push_str(s);
    }

    fn term(&mut self, t: &Term, model: &Model) {
        let start = self.text.len();
        self.text.push_str(&t.print(model));
        self.terms.push(start..self.text.len());
        self.arms.extend(
            t.arm_ranges(model)
                .into_iter()
                .map(|(k, r)| (k, start + r.start..start + r.end)),
        );
    }

    fn list(&mut self, ts: &[Term], model: &Model, sep: &str) {
        for (k, t) in ts.iter().enumerate() {
            if k > 0 {
                self.push(sep);
            }
            self.term(t, model);
        }
    }
}

impl Model {
    /// Returns the text of declaration `i`, without its module header.
    pub(super) fn print(&self, i: usize) -> String {
        self.print_mapped(i).text
    }

    /// Returns the text of declaration `i`, without its module header, with the range each of
    /// its terms takes.
    pub(super) fn print_mapped(&self, i: usize) -> Printed {
        let mut out = Printed::default();
        match self.decls[i].item {
            Item::Type(t) => self.print_type(t, &mut out),
            Item::Relation(r) => self.print_relation(r, &mut out),
            Item::Set(s) => {
                let set = &self.sets[&s];
                out.push(&format!(
                    "set {} of {} = ",
                    set.name, self.relations[&set.relation].name
                ));
                out.list(&set.members, self, " | ");
            }
            Item::Fn(f) => self.print_body("fn", &self.functions[&f], &mut out),
            Item::Aggregate(f) => self.print_body("aggregate", &self.functions[&f], &mut out),
            Item::Constraint(c) => self.print_body("constraint", &self.denials[&c], &mut out),
            Item::Check(c) => self.print_body("check", &self.denials[&c], &mut out),
            Item::Store(v) => {
                out.push(&format!("store {}", self.store_name(v)));
                out.term(&self.stores[&v].rest, self);
            }
            Item::Held(_, k) => out.push(&self.held[&k].text),
        }
        out
    }

    /// Returns the terms of declaration `i`, in the order [`Model::print_mapped`] prints them.
    pub(super) fn decl_terms_mut(&mut self, i: usize) -> Vec<&mut Term> {
        let mut out: Vec<&mut Term> = Vec::new();
        match self.decls[i].item {
            Item::Type(t) => {
                let ty = &self.types[&t];
                let header: Vec<u64> = ty.header.clone().unwrap_or_default();
                let ctors = match &ty.body {
                    TypeBody::Record { ctor } => vec![*ctor],
                    TypeBody::Ctors(cs) => cs.clone(),
                    TypeBody::New { .. } => Vec::new(),
                };
                let own: Vec<u64> = ctors
                    .iter()
                    .flat_map(|c| self.ctors[c].fields.iter().flatten().copied())
                    .collect();
                let mut held: BTreeMap<u64, &mut FieldM> = self
                    .fields
                    .iter_mut()
                    .filter(|(f, _)| header.contains(f) || own.contains(f))
                    .map(|(&f, x)| (f, x))
                    .collect();
                let mut new_terms: Vec<&mut Term> = Vec::new();
                if let TypeBody::New { base, domain, .. } =
                    &mut self.types.get_mut(&t).expect("the type").body
                {
                    new_terms.push(base);
                    new_terms.extend(domain.iter_mut());
                }
                for (k, f) in header.iter().chain(&own).enumerate() {
                    if k == header.len() {
                        out.append(&mut new_terms);
                    }
                    let x = held.remove(f).expect("a printed field");
                    out.push(&mut x.ty);
                    if let Some(a) = &mut x.action {
                        out.push(a);
                    }
                }
                out.append(&mut new_terms);
            }
            Item::Relation(r) => {
                let rel = self.relations.get_mut(&r).expect("the relation variable");
                out.push(&mut rel.ty);
                for c in &mut rel.clauses {
                    match c {
                        ClauseM::Key { fields, .. } => out.extend(fields.iter_mut()),
                        ClauseM::Index { elems, scope, .. } => {
                            out.extend(elems.iter_mut());
                            if let Some(s) = scope {
                                out.extend(s.parts.iter_mut());
                            }
                        }
                        ClauseM::Default {
                            ctor, field, value, ..
                        } => {
                            out.extend(ctor.iter_mut());
                            out.push(field);
                            out.push(value);
                        }
                        ClauseM::Computed { ty, expr, .. } => {
                            out.push(ty);
                            out.push(expr);
                        }
                        ClauseM::Volatile => {}
                    }
                }
            }
            Item::Set(s) => out.extend(self.sets.get_mut(&s).expect("the set").members.iter_mut()),
            Item::Fn(f) | Item::Aggregate(f) => {
                out.push(&mut self.functions.get_mut(&f).expect("the function").rest);
            }
            Item::Constraint(c) | Item::Check(c) => {
                out.push(&mut self.denials.get_mut(&c).expect("the constraint").rest);
            }
            Item::Store(v) => out.push(&mut self.stores.get_mut(&v).expect("the view").rest),
            Item::Held(..) => {}
        }
        out
    }

    fn print_body(&self, kw: &str, b: &Body, out: &mut Printed) {
        out.push(&format!("{kw} {}", b.name));
        out.term(&b.rest, self);
    }

    fn print_field(&self, f: u64, out: &mut Printed) {
        let f = &self.fields[&f];
        out.push(&format!("{}: ", f.name));
        out.term(&f.ty, self);
        if let Some(a) = &f.action {
            out.push(" on delete ");
            out.term(a, self);
        }
    }

    fn print_record(&self, fields: &[u64], out: &mut Printed) {
        if fields.is_empty() {
            out.push("{}");
            return;
        }
        out.push("{ ");
        for (k, &f) in fields.iter().enumerate() {
            if k > 0 {
                out.push(", ");
            }
            self.print_field(f, out);
        }
        out.push(" }");
    }

    fn print_type(&self, t: u64, out: &mut Printed) {
        let ty = &self.types[&t];
        out.push(&format!("type {}", ty.name));
        for p in &ty.params {
            out.push(" ");
            out.push(p);
        }
        if ty.ordered {
            out.push(" ordered");
        }
        if let Some(h) = &ty.header {
            out.push(" ");
            self.print_record(h, out);
        }
        out.push(" = ");
        match &ty.body {
            TypeBody::New { base, domain, .. } => {
                out.push("new ");
                out.term(base, self);
                if let Some(d) = domain {
                    out.push(" where ");
                    out.term(d, self);
                }
            }
            TypeBody::Record { ctor } => {
                let fs = self.ctors[ctor].fields.clone().unwrap_or_default();
                self.print_record(&fs, out);
            }
            TypeBody::Ctors(cs) => {
                for (k, c) in cs.iter().enumerate() {
                    if k > 0 {
                        out.push(" | ");
                    }
                    let c = &self.ctors[c];
                    out.push(&c.name);
                    if let Some(fs) = &c.fields {
                        out.push(" ");
                        self.print_record(fs, out);
                    }
                }
            }
        }
    }

    fn print_relation(&self, r: u64, out: &mut Printed) {
        let rel = &self.relations[&r];
        out.push(&format!("relation {}: ", rel.name));
        out.term(&rel.ty, self);
        for c in &rel.clauses {
            out.push("\n  ");
            self.print_clause(c, out);
        }
    }

    fn print_clause(&self, c: &ClauseM, out: &mut Printed) {
        match c {
            ClauseM::Key {
                fields,
                listed,
                generate,
                ..
            } => {
                out.push(if *listed { "key (" } else { "key " });
                out.list(fields, self, ", ");
                if *listed {
                    out.push(")");
                }
                if let Some((g, _)) = generate {
                    out.push(" generate ");
                    out.push(g);
                }
            }
            ClauseM::Volatile => out.push("volatile"),
            ClauseM::Index {
                kind, elems, scope, ..
            } => {
                out.push(match kind {
                    IndexKind::Unique => "unique (",
                    IndexKind::Plain => "index (",
                    IndexKind::Interval => "interval index (",
                });
                out.list(elems, self, ", ");
                out.push(")");
                if let Some(s) = scope {
                    out.push(" in ");
                    out.push(if s.listed { "(" } else { "" });
                    out.list(&s.parts, self, " | ");
                    out.push(if s.listed { ")" } else { "" });
                }
            }
            ClauseM::Default {
                ctor, field, value, ..
            } => {
                out.push("default ");
                if let Some(c) = ctor {
                    out.term(c, self);
                    out.push(".");
                }
                out.term(field, self);
                out.push(" = ");
                out.term(value, self);
            }
            ClauseM::Computed { name, ty, expr, .. } => {
                out.push(&format!("computed {name}: "));
                out.term(ty, self);
                out.push(" = ");
                out.term(expr, self);
            }
        }
    }

    /// Returns the declarations as [`Model::source_decls`] does, a name of dropped objects as
    /// written, for a build while a migration applies.
    pub(super) fn printed(&self) -> Result<Vec<SourceDecl>, String> {
        Ok((0..self.decls.len())
            .map(|i| SourceDecl {
                module: self.decls[i].module.clone(),
                text: self.print(i),
            })
            .collect())
    }

    /// Returns the files of the declarations, declaration `i` file `i`, then one file of each of
    /// `more`, and a file of each module that holds nothing.
    pub(super) fn files(
        &self,
        more: &[SourceDecl],
    ) -> Result<Vec<(FileId, ast::SchemaFile)>, Vec<FileDiag>> {
        let mut out = Vec::new();
        let mut errs = Vec::new();
        let held = self.printed().map_err(|why| {
            vec![FileDiag {
                file: FileId::EMPTY_MODULE,
                diag: crate::span::Diag::new(Span::new(0, 0), why),
            }]
        })?;
        let all = held.into_iter().chain(more.iter().cloned());
        for (i, d) in all.enumerate() {
            let text = format!("module {}\n{}", d.module, d.text);
            match parse_schema(&text, Options { elisions: true }).ok() {
                Ok(f) => out.push((FileId(i as u32), f)),
                Err(ds) => errs.extend(ds.into_iter().map(|diag| FileDiag {
                    file: FileId(i as u32),
                    diag,
                })),
            }
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        for m in &self.modules {
            if !out.iter().any(|(_, f)| f.module.text == *m) {
                let empty = parse_schema(&format!("module {m}\n"), Options::default())
                    .ok()
                    .expect("a module header parses");
                out.push((FileId::EMPTY_MODULE, empty));
            }
        }
        Ok(out)
    }

    /// Returns the write states as a schema takes them.
    pub(super) fn schema_states(&self) -> schema::States {
        let mut states = schema::States::default();
        for (&(r, c), s) in &self.states {
            states.set_relation(
                crate::ids::CatRelId::from_raw(r),
                crate::ids::CatCtorId::from_raw(c),
                s.stage,
            );
        }
        for (&c, s) in &self.value_states {
            states.set_value(crate::ids::CatCtorId::from_raw(c), s.stage);
        }
        states
    }
}
