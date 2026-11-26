use super::*;

/// Returns the program of every body of `schema`, each function and aggregate checked before
/// the first body that names it, with the diagnostics of the bodies that do not check. With
/// `record`, appends the schema objects each body names.
pub(super) fn bodies(
    schema: &Schema,
    record: Option<&mut Vec<(FileId, Span, Ref)>>,
) -> (Program, Vec<FileDiag>) {
    let mut b = Bodies {
        schema,
        record,
        program: Program {
            terms: Terms::default(),
            functions: vec![None; schema.functions.len()],
            constraints: BTreeMap::new(),
            checks: BTreeMap::new(),
            domains: BTreeMap::new(),
            retags: BTreeMap::new(),
            defaults: BTreeMap::new(),
            computed: BTreeMap::new(),
            index_fields: BTreeMap::new(),
            aggregates: vec![None; schema.aggregates.len()],
            fn_rows: Vec::new(),
        },
        diags: Vec::new(),
        fn_rows: vec![None; schema.functions.len()],
        terms: Terms::default(),
        demand: Rc::default(),
    };
    for i in 0..schema.functions.len() {
        b.demand(Demanded::Function(FnId(i as u32)));
    }
    for i in 0..schema.aggregates.len() {
        b.demand(Demanded::Aggregate(i as u32));
    }
    b.constraints();
    b.checks();
    b.domains();
    for (i, r) in schema.relations.iter().enumerate() {
        b.relation(RelId(i as u32), r);
    }
    b.finish()
}

pub(super) struct Bodies<'s, 'r> {
    schema: &'s Schema,
    record: Option<&'r mut Vec<(FileId, Span, Ref)>>,
    program: Program,
    diags: Vec<FileDiag>,
    fn_rows: Vec<Option<RelId>>,
    terms: Terms,
    demand: Rc<Demand>,
}

/// A function or an aggregate, whose body the checker checks before a body that names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Demanded {
    Function(FnId),
    Aggregate(u32),
}

/// Which functions and aggregates of a schema are checked, and what the body being checked names.
#[derive(Debug, Default)]
pub(super) struct Demand {
    /// Each function and aggregate whose check began, `true` once it ended.
    started: RefCell<BTreeMap<Demanded, bool>>,
    /// What the body being checked names that no check began on.
    pending: RefCell<BTreeSet<Demanded>>,
    /// Each function and aggregate a body named while its own check was under way.
    cycles: RefCell<BTreeSet<Demanded>>,
}

impl Demand {
    /// Record that the body being checked names `d`.
    pub(super) fn named(&self, d: Demanded) {
        match self.started.borrow().get(&d) {
            None => {
                self.pending.borrow_mut().insert(d);
            }
            Some(false) => {
                self.cycles.borrow_mut().insert(d);
            }
            Some(true) => {}
        }
    }
}

impl<'s> Bodies<'s, '_> {
    pub(super) fn schema(&self) -> &'s Schema {
        self.schema
    }

    pub(super) fn refuse(&mut self, d: FileDiag) {
        self.diags.push(d);
    }

    /// Returns what `f` gives checking a body of `module` in `file`, checking first each function
    /// and aggregate it names that is not checked; `None` with its diagnostic when it fails.
    pub(super) fn run<T>(
        &mut self,
        module: ModuleId,
        ctx: Context,
        file: FileId,
        f: &mut dyn FnMut(&mut Checker) -> R<T>,
    ) -> Option<T> {
        loop {
            let mut c = Checker::with_rows(
                self.schema,
                Visible::module(module),
                ctx,
                self.fn_rows.clone(),
            );
            c.demand = Some(Rc::clone(&self.demand));
            c.terms = std::mem::take(&mut self.terms);
            if self.record.is_some() {
                c.coverage = false;
            }
            let r = f(&mut c);
            self.terms = std::mem::take(&mut c.terms);
            let pending = std::mem::take(&mut *self.demand.pending.borrow_mut());
            if pending.is_empty() {
                return self.ran(c, file, r);
            }
            for d in pending {
                self.demand(d);
            }
        }
    }

    fn ran<T>(&mut self, c: Checker, file: FileId, r: R<T>) -> Option<T> {
        if let Some(out) = self.record.as_deref_mut() {
            out.extend(c.refs.take_in(file));
        }
        match r {
            Ok(t) => Some(t),
            Err(d) => {
                self.diags.push(FileDiag { file, diag: d });
                None
            }
        }
    }

    /// Check the body of `d` unless its check began.
    fn demand(&mut self, d: Demanded) {
        if self.demand.started.borrow().contains_key(&d) {
            return;
        }
        self.demand.started.borrow_mut().insert(d, false);
        match d {
            Demanded::Function(id) => self.function(id),
            Demanded::Aggregate(i) => self.aggregate(i as usize),
        }
        self.demand.started.borrow_mut().insert(d, true);
    }

    fn function(&mut self, id: FnId) {
        let def = self.schema.function(id);
        let ast::FnBody::Expr(body) = &def.body else {
            return;
        };
        let mut rows = None;
        let tm = self.run(def.module, Context::Schema, def.file, &mut |c| {
            let (tm, r) = c.function(def, body)?;
            rows = r;
            Ok(tm)
        });
        self.fn_rows[id.0 as usize] = rows;
        self.program.functions[id.0 as usize] = tm;
    }

    fn aggregate(&mut self, i: usize) {
        let def = &self.schema.aggregates[i];
        let mut parts = None;
        self.run(def.module, Context::Schema, def.file, &mut |c| {
            let p = c.aggregate_decl(def)?;
            parts = Some(p);
            Ok(p.init)
        });
        self.program.aggregates[i] = parts;
    }

    /// Returns the refusal of each function and aggregate a body named while its own check was
    /// under way.
    fn recursion(&self) -> Vec<FileDiag> {
        let schema = self.schema;
        let refused = |kind: &str, name: &str, file: FileId, span: Span| FileDiag {
            file,
            diag: diag(
                span,
                format!("{kind} `{name}` calls itself; recursion is written with `fix`"),
            ),
        };
        self.demand
            .cycles
            .borrow()
            .iter()
            .map(|&d| match d {
                Demanded::Function(id) => {
                    let f = schema.function(id);
                    refused("function", &f.name, f.file, f.span)
                }
                Demanded::Aggregate(i) => {
                    let a = &schema.aggregates[i as usize];
                    refused("aggregate", &a.name, a.file, a.decl.name.span)
                }
            })
            .collect()
    }

    fn constraints(&mut self) {
        let schema = self.schema;
        for (i, con) in schema.constraint_defs() {
            if let Some(tm) = self.run(con.module, Context::Schema, con.file, &mut |c| {
                c.constraint(&con.query)
            }) {
                self.program.constraints.insert(i, tm);
            }
        }
    }

    fn checks(&mut self) {
        let schema = self.schema;
        for (i, chk) in schema.check_defs() {
            if let Some(tm) = self.run(chk.module, Context::Schema, chk.file, &mut |c| {
                c.check_clause(chk.relation, chk.ctors.clone(), &chk.predicate)
            }) {
                self.program.checks.insert(i, tm);
            }
        }
    }

    fn domains(&mut self) {
        let schema = self.schema;
        for (i, t) in schema.types.iter().enumerate() {
            if let TypeDefKind::Newtype {
                base,
                domain: Some(d),
            } = &t.kind
            {
                let mut var = None;
                if let Some(tm) = self.run(t.module, Context::Schema, t.file, &mut |c| {
                    c.scope.push();
                    let v = c.bind("self", base.clone(), None, Provenance::Built);
                    var = Some(v);
                    let ty = c.bool_ty();
                    let out = c.check(d, &ty)?;
                    Ok(out.tm)
                }) {
                    self.program
                        .domains
                        .insert(TypeId(i as u32), (var.expect("bound"), tm));
                }
            }
        }
    }

    fn relation(&mut self, rel: RelId, r: &crate::schema::RelationDef) {
        let mut checked: Vec<(Span, Ty, Option<Tm>)> = Vec::new();
        for (j, d) in r.defaults.iter().enumerate() {
            let DefaultValue::Expr(e) = &d.value else {
                continue;
            };
            let fty = self.schema.ty(r.elem_def).field_def(d.field).ty.clone();
            let tm = match checked
                .iter()
                .find(|(s, t, _)| *s == d.written && *t == fty)
            {
                Some(&(.., tm)) => tm,
                None => {
                    let tm = self.run(r.module, Context::Schema, r.file, &mut |c| {
                        c.default_clause(rel, d.field, e)
                    });
                    checked.push((d.written, fty, tm));
                    tm
                }
            };
            if let Some(tm) = tm {
                self.program.defaults.insert((rel, j), tm);
            }
        }
        for (j, comp) in r.computed.iter().enumerate() {
            let mut var = None;
            if let Some(tm) = self.run(r.module, Context::Schema, r.file, &mut |c| {
                c.scope.push();
                var = Some(c.bind("self", r.elem.clone(), Some(rel), Provenance::Snapshot));
                Ok(c.check(&comp.expr, &comp.ty)?.tm)
            }) {
                self.program
                    .computed
                    .insert((rel, j), (var.expect("bound"), tm));
            }
        }
        for (j, ix) in r.indexes.iter().enumerate() {
            for (k, e) in ix.fields.iter().enumerate() {
                let mut path = Vec::new();
                if let Some(value) = self.run(r.module, Context::Schema, r.file, &mut |c| {
                    let (f, ty) = c.index_field(rel, ix.scope.clone(), e)?;
                    c.keyable(rel, ix, k, &ty)?;
                    path = f.path;
                    Ok(f.value)
                }) {
                    self.program
                        .index_fields
                        .insert((rel, j, k), IndexField { value, path });
                }
            }
        }
    }

    fn finish(mut self) -> (Program, Vec<FileDiag>) {
        let mut diags = self.recursion();
        diags.append(&mut self.diags);
        self.diags = diags;
        self.program.retags = write::delete_actions(&mut self);
        self.program.terms = self.terms;
        self.program.fn_rows = self.fn_rows;
        (self.program, self.diags)
    }
}
