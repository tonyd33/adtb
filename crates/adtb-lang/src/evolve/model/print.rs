use super::*;
use crate::ast;
use crate::evolve::SourceDecl;
use crate::parse::{Options, parse_schema};
use crate::schema;
use crate::span::{FileDiag, FileId, Span};

impl Model {
    /// Returns the text of declaration `i`, without its module header.
    pub(super) fn print(&self, i: usize) -> String {
        match self.decls[i].item {
            Item::Type(t) => self.print_type(t),
            Item::Relation(r) => self.print_relation(r),
            Item::Set(s) => {
                let set = &self.sets[&s];
                let members: Vec<String> = set.members.iter().map(|c| c.print(self)).collect();
                format!(
                    "set {} of {} = {}",
                    set.name,
                    self.relations[&set.relation].name,
                    members.join(" | ")
                )
            }
            Item::Fn(f) => self.print_body("fn", &self.functions[&f]),
            Item::Aggregate(f) => self.print_body("aggregate", &self.functions[&f]),
            Item::Constraint(c) => self.print_body("constraint", &self.denials[&c]),
            Item::Check(c) => self.print_body("check", &self.denials[&c]),
            Item::Store(v) => {
                let st = &self.stores[&v];
                format!("store {}{}", self.store_name(v), st.rest.print(self))
            }
        }
    }

    fn print_body(&self, kw: &str, b: &Body) -> String {
        format!("{kw} {}{}", b.name, b.rest.print(self))
    }

    fn print_field(&self, f: u64) -> String {
        let f = &self.fields[&f];
        let mut out = format!("{}: {}", f.name, f.ty.print(self));
        if let Some(a) = &f.action {
            out.push_str(" on delete ");
            out.push_str(&a.print(self));
        }
        out
    }

    fn print_record(&self, fields: &[u64]) -> String {
        if fields.is_empty() {
            return "{}".to_string();
        }
        let fs: Vec<String> = fields.iter().map(|&f| self.print_field(f)).collect();
        format!("{{ {} }}", fs.join(", "))
    }

    fn print_type(&self, t: u64) -> String {
        let ty = &self.types[&t];
        let mut out = format!("type {}", ty.name);
        for p in &ty.params {
            out.push(' ');
            out.push_str(p);
        }
        if ty.ordered {
            out.push_str(" ordered");
        }
        if let Some(h) = &ty.header {
            out.push(' ');
            out.push_str(&self.print_record(h));
        }
        out.push_str(" = ");
        match &ty.body {
            TypeBody::New { base, domain, .. } => {
                out.push_str("new ");
                out.push_str(&base.print(self));
                if let Some(d) = domain {
                    out.push_str(" where ");
                    out.push_str(&d.print(self));
                }
            }
            TypeBody::Record { ctor } => {
                let fs = self.ctors[ctor].fields.clone().unwrap_or_default();
                out.push_str(&self.print_record(&fs));
            }
            TypeBody::Ctors(cs) => {
                let cs: Vec<String> = cs
                    .iter()
                    .map(|c| {
                        let c = &self.ctors[c];
                        match &c.fields {
                            Some(fs) => format!("{} {}", c.name, self.print_record(fs)),
                            None => c.name.clone(),
                        }
                    })
                    .collect();
                out.push_str(&cs.join(" | "));
            }
        }
        out
    }

    fn print_relation(&self, r: u64) -> String {
        let rel = &self.relations[&r];
        let mut out = format!("relation {}: {}", rel.name, rel.ty.print(self));
        for c in &rel.clauses {
            out.push_str("\n  ");
            out.push_str(&self.print_clause(c));
        }
        out
    }

    fn print_clause(&self, c: &ClauseM) -> String {
        let list = |ts: &[Term]| {
            ts.iter()
                .map(|t| t.print(self))
                .collect::<Vec<_>>()
                .join(", ")
        };
        match c {
            ClauseM::Key {
                fields,
                listed,
                generate,
                ..
            } => {
                let mut out = if *listed {
                    format!("key ({})", list(fields))
                } else {
                    format!("key {}", list(fields))
                };
                if let Some((g, _)) = generate {
                    out.push_str(" generate ");
                    out.push_str(g);
                }
                out
            }
            ClauseM::Volatile => "volatile".to_string(),
            ClauseM::Index {
                kind, elems, scope, ..
            } => {
                let kw = match kind {
                    IndexKind::Unique => "unique",
                    IndexKind::Plain => "index",
                    IndexKind::Interval => "interval index",
                };
                let mut out = format!("{kw} ({})", list(elems));
                if let Some(s) = scope {
                    out.push_str(" in ");
                    out.push_str(&self.print_scope(s));
                }
                out
            }
            ClauseM::Default {
                ctor, field, value, ..
            } => match ctor {
                Some(c) => format!(
                    "default {}.{} = {}",
                    c.print(self),
                    field.print(self),
                    value.print(self)
                ),
                None => format!("default {} = {}", field.print(self), value.print(self)),
            },
            ClauseM::Computed { name, ty, expr, .. } => {
                format!("computed {name}: {} = {}", ty.print(self), expr.print(self))
            }
        }
    }

    fn print_scope(&self, s: &Scope) -> String {
        let parts: Vec<String> = s.parts.iter().map(|p| p.print(self)).collect();
        if s.listed {
            format!("({})", parts.join(" | "))
        } else {
            parts.join(" | ")
        }
    }

    /// Returns the declarations as a source holds them, each printed.
    ///
    /// Fail when a declaration holds a name of dropped objects no operator added again.
    pub(super) fn source_decls(&self) -> Result<Vec<SourceDecl>, String> {
        if self.dropped_names() {
            return Err("a declaration names a dropped object".into());
        }
        self.printed()
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
