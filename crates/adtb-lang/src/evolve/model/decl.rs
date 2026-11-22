use super::lift::Lifting;
use super::op::{Fail, Kind, Res, StateKey, broken, refuse, refuse_at, unmodelled};
use super::run::{Built, Late, Run, unbuilt};
use super::slots::Found;
use super::*;
use crate::ast::{self, OperatorKind};
use crate::check;
use crate::evolve::{Clause, Resolved, SourceDecl, Target, mentions_def, mentions_rel};
use crate::ids::named_ids;
use crate::schema::{self, PRELUDE, Stage};
use crate::span::{FileId, Span};

impl Model {
    /// Refuse a module named `m` added: a built-in module's name, or one taken.
    pub(super) fn reserved(&self, m: &ast::Ident) -> Res<()> {
        let Ok(schema) = schema::build(&[]) else {
            return broken("the built-in modules do not build");
        };
        let built_in = schema
            .modules
            .iter()
            .enumerate()
            .any(|(i, d)| (i == PRELUDE.0 as usize || d.engine) && d.name == m.text);
        if built_in {
            return refuse_at(m.span, format!("no module may be named `{}`", m.text));
        }
        if self.modules.contains(&m.text) {
            return refuse_at(m.span, format!("module `{}` already exists", m.text));
        }
        Ok(())
    }

    pub(super) fn add_module(&mut self, m: &ast::Ident) -> Res<(Resolved, Vec<Clause>)> {
        self.reserved(m)?;
        self.modules.insert(m.text.clone());
        Ok((
            Resolved::AddModule {
                module: m.text.clone(),
            },
            Vec::new(),
        ))
    }

    pub(super) fn drop_module(&mut self, m: &ast::Ident) -> Res<(Resolved, Vec<Clause>)> {
        let (span, m) = (m.span, m.text.clone());
        if !self.modules.contains(&m) {
            return refuse_at(span, format!("no module `{m}`"));
        }
        if let Some(i) = self.decls.iter().position(|d| d.module == m) {
            return refuse_at(
                span,
                format!(
                    "`drop module {m}` is refused while the module holds {}",
                    self.shown_decl(i)
                ),
            );
        }
        self.modules.remove(&m);
        self.installed.remove(&m);
        Ok((Resolved::DropModule { module: m }, Vec::new()))
    }

    pub(super) fn add(
        &mut self,
        run: &Run,
        kind: Kind,
        name: &str,
        span: Span,
    ) -> Res<(Resolved, Vec<Clause>)> {
        if !self.modules.contains(run.module()) {
            return refuse(format!(
                "no module `{0}`; a migration of a new module starts with `add module {0}`",
                run.module()
            ));
        }
        if self.find(run.module(), kind, name).is_some() {
            return refuse_at(
                span,
                format!(
                    "module `{}` already declares {} `{name}`",
                    run.module(),
                    kind.word()
                ),
            );
        }
        let at = FileId(self.decls.len() as u32);
        let decl = SourceDecl {
            module: run.module().to_string(),
            text: run.text()[span.range()].to_string(),
        };
        let built = match self.build_adopting(std::slice::from_ref(&decl), &run.ahead(self)) {
            Ok(b) => b,
            Err(ds) => {
                if let Some(d) = ds.iter().find(|d| d.file == at) {
                    return refuse(d.diag.message.clone());
                }
                if kind == Kind::Relation {
                    return unbuilt(&ds);
                }
                return Ok(self.hold(run, kind, name, decl.text));
            }
        };
        if let Some(e) = built.errors_in(at).into_iter().next() {
            return refuse(e);
        }
        let Some(d) = built.decl(at) else {
            return broken("an added declaration that does not parse alone");
        };
        let names = built.names(at);
        let ahead = built.ahead_ranges(at);
        Lifting::new(
            built.schema(),
            at,
            built.text(at),
            &names,
            &ahead,
            run.module(),
        )
        .decl(self, d)
        .map_err(Fail::Broken)?;
        let item = self.decls.last().expect("the added declaration").item;
        let decl = match built.canonical(at)? {
            Some(decl) => decl,
            None => run.late(Late::Decl(item)),
        };
        self.enable(item, &built, at);
        Ok(self.added(item, decl))
    }

    /// Hold declaration `text` of kind `kind` named `name`, added while the declarations do not
    /// build, until they build. Returns the operator resolved as the migration's end resolves it.
    fn hold(&mut self, run: &Run, kind: Kind, name: &str, text: String) -> (Resolved, Vec<Clause>) {
        let key = self.hold_text(run.module(), kind, name, text, true);
        let decl = run.late(Late::Held(key));
        (Resolved::AddModule { module: decl }, Vec::new())
    }

    /// Hold declaration `text` of kind `kind` named `name` in module `module` after the others
    /// until the declarations build, giving it its first write states then when `enabling`.
    /// Returns its key in [`Model::held`].
    pub(super) fn hold_text(
        &mut self,
        module: &str,
        kind: Kind,
        name: &str,
        text: String,
        enabling: bool,
    ) -> u64 {
        let key = self
            .held
            .keys()
            .chain(self.lifted.keys())
            .max()
            .map_or(0, |k| k + 1);
        self.held.insert(
            key,
            HeldM {
                name: name.to_string(),
                text,
                enabling,
            },
        );
        self.decls.push(Decl {
            module: module.to_string(),
            item: Item::Held(kind, key),
        });
        if let Ok(files) = self.files(&[]) {
            let known = self.named_keys();
            let given = named_ids(&files, &known, &mut self.next, &mut BTreeSet::new());
            self.held_ids
                .extend(given.into_iter().filter(|(k, _)| !known.contains_key(k)));
        }
        key
    }

    /// Give each constructor declaration `item`, of file `at` of `built`, declares its first write
    /// states: a type's its value-level ones, a relation variable's those of its row type's.
    pub(super) fn enable(&mut self, item: Item, built: &Built, at: FileId) {
        let since = self.version;
        match item {
            Item::Type(t) if !matches!(self.types[&t].body, TypeBody::New { .. }) => {
                for c in self.ctors_of(t) {
                    self.value_states.insert(
                        c,
                        State {
                            stage: Stage::Enabled,
                            since,
                        },
                    );
                }
            }
            Item::Relation(r) => {
                let schema = built.schema();
                let Some(rel) = Found::new(schema, at).relation() else {
                    return;
                };
                let td = schema.ty(schema.relation(rel).elem_def);
                if !matches!(td.kind, schema::TypeDefKind::Sum { .. }) {
                    return;
                }
                for c in td.ctor_refs() {
                    let Some(ctor) = schema.ids().ctor_id(c) else {
                        continue;
                    };
                    let stage = self
                        .value_states
                        .get(&ctor.raw())
                        .map_or(Stage::Enabled, |s| s.stage);
                    self.states.insert((r, ctor.raw()), State { stage, since });
                }
            }
            _ => {}
        }
    }

    /// Returns the operator that added declaration `item`, whose canonical form is `decl`,
    /// resolved.
    pub(super) fn added(&self, item: Item, decl: String) -> (Resolved, Vec<Clause>) {
        let mut clauses = Vec::new();
        let resolved = match item {
            Item::Type(t) => Resolved::AddType { sum: t, decl },
            Item::Relation(r) => Resolved::AddRelation { relation: r, decl },
            Item::Set(s) => {
                let set = &self.sets[&s];
                for m in &set.members {
                    if let Some((Space::Shared, c)) = m.objects().next() {
                        clauses.push(Clause::Member { ctor: c });
                    }
                }
                Resolved::AddSet {
                    set: s,
                    relation: set.relation,
                    name: set.name.clone(),
                }
            }
            Item::Fn(f) => Resolved::DefineFn { function: f, decl },
            Item::Aggregate(f) => Resolved::DefineAggregate { function: f, decl },
            Item::Constraint(c) => Resolved::AddConstraint {
                constraint: c,
                body: decl,
            },
            Item::Check(c) => Resolved::AddCheck {
                constraint: c,
                body: decl,
            },
            Item::Store(v) => Resolved::Store {
                view: v,
                function: self.stores[&v].function,
                decl,
            },
            Item::Held(..) => Resolved::AddModule { module: decl },
        };
        (resolved, clauses)
    }

    pub(super) fn redefine_fn(
        &mut self,
        run: &Run,
        function: u64,
        span: Span,
        op: &OperatorKind,
    ) -> Res<(Resolved, Vec<Clause>)> {
        let OperatorKind::RedefineFn(f) = op else {
            return broken("a redefinition of no function");
        };
        let at = self
            .decl_at(Item::Fn(function))
            .expect("a function the operator resolved");
        let rest = &run.text()[f.name.span.end as usize..span.end as usize];
        self.functions
            .get_mut(&function)
            .expect("the function")
            .rest = Term::text(rest);
        let built = match self.build_adopting(&[], &run.ahead(self)) {
            Ok(b) => b,
            Err(ds) => {
                if let Some(d) = ds.iter().find(|d| d.file.0 as usize == at) {
                    return refuse(d.diag.message.clone());
                }
                let item = Item::Fn(function);
                self.unlifted.push(item);
                let decl = run.late(Late::Decl(item));
                return Ok((Resolved::RedefineFn { function, decl }, Vec::new()));
            }
        };
        let file = FileId(at as u32);
        if let Some(e) = built.errors_in(file).into_iter().next() {
            return refuse(e);
        }
        let Some(ast::Decl::Fn(d)) = built.decl(file) else {
            return broken("a redefined function that does not parse alone");
        };
        let names = built.names(file);
        let ahead = built.ahead_ranges(file);
        let lifting = Lifting::new(
            built.schema(),
            file,
            built.text(file),
            &names,
            &ahead,
            run.module(),
        );
        let body = lifting.body(&d.name, d.span);
        self.functions.insert(function, body);
        Ok((
            Resolved::RedefineFn {
                function,
                decl: built
                    .canonical(file)?
                    .ok_or(Fail::Broken("a redefined function does not resolve".into()))?,
            },
            Vec::new(),
        ))
    }

    pub(super) fn drop(&mut self, run: &Run, item: Item) -> Res<(Resolved, Vec<Clause>)> {
        let resolved = match item {
            Item::Type(t) => {
                self.drop_type_allowed(run, t)?;
                Resolved::DropType { sum: t }
            }
            Item::Relation(r) => {
                self.drop_relation_allowed(run, r)?;
                Resolved::DropRelation { relation: r }
            }
            Item::Set(s) => Resolved::DropSet { set: s },
            Item::Fn(f) => Resolved::DropFn { function: f },
            Item::Aggregate(f) => Resolved::DropAggregate { function: f },
            Item::Constraint(c) => Resolved::DropConstraint { constraint: c },
            Item::Check(c) => Resolved::DropCheck { constraint: c },
            Item::Store(v) => Resolved::Unstore { view: v },
            Item::Held(..) => {
                return unmodelled(
                    "a drop of a declaration held while the declarations do not build",
                );
            }
        };
        let mut gone = vec![item];
        if let Item::Relation(r) = item {
            gone.extend(self.sets_of(r).into_iter().map(Item::Set));
        }
        for g in gone {
            if let Item::Set(s) = g {
                self.unscope(s);
            }
            self.remove(g);
        }
        Ok((resolved, Vec::new()))
    }

    /// Leave each unique and index whose scope names set `s` without an id, so that the next
    /// build gives it a new one.
    fn unscope(&mut self, s: u64) {
        for r in self.relations.values_mut() {
            for c in &mut r.clauses {
                if let ClauseM::Index {
                    id,
                    scope: Some(scope),
                    ..
                } = c
                    && scope
                        .parts
                        .iter()
                        .any(|p| p.objects().any(|o| o == (Space::Set, s)))
                {
                    *id = None;
                }
            }
        }
    }

    /// Remove declaration `item` and every object it declares.
    pub(super) fn remove(&mut self, item: Item) {
        self.decls.retain(|d| d.item != item);
        match item {
            Item::Type(t) => {
                for f in self.fields_of(t) {
                    self.fields.remove(&f);
                }
                for c in self.ctors_of(t) {
                    self.ctors.remove(&c);
                }
                self.types.remove(&t);
            }
            Item::Relation(r) => {
                self.relations.remove(&r);
            }
            Item::Set(s) => {
                self.sets.remove(&s);
            }
            Item::Fn(f) | Item::Aggregate(f) => {
                self.functions.remove(&f);
            }
            Item::Constraint(c) | Item::Check(c) => {
                self.denials.remove(&c);
            }
            Item::Store(v) => {
                self.stores.remove(&v);
            }
            Item::Held(_, k) => {
                self.held.remove(&k);
            }
        }
    }

    /// Refuse dropping type `t` while a declaration uses it. While the declarations do not
    /// build, allow it only when they build without it.
    fn drop_type_allowed(&self, run: &Run, t: u64) -> Res<()> {
        let built = match self.schema_now(run) {
            Ok(b) => b,
            Err(e) => {
                let mut without = self.clone();
                without.remove(Item::Type(t));
                return match without.built(&[], &run.ahead(&without)) {
                    Ok(_) => Ok(()),
                    Err(_) => Err(e),
                };
            }
        };
        let schema = built.schema();
        let Some(def) = schema.ids().type_of(crate::ids::CatTypeId::from_raw(t)) else {
            return broken("a dropped type the schema does not hold");
        };
        let name = &self.types[&t].name;
        let refused = |what: String| {
            refuse(format!(
                "`drop type {name}` is refused while {what} uses it"
            ))
        };
        for r in &schema.relations {
            if r.elem_def == def {
                return refused(format!("relation `{}`", r.name));
            }
        }
        for (j, td) in schema.types.iter().enumerate() {
            if j == def.0 as usize || schema.module(td.module).engine {
                continue;
            }
            for f in td
                .header()
                .iter()
                .chain(td.ctors().iter().flat_map(|c| c.fields.iter()))
            {
                if mentions_def(&f.ty, def) {
                    return refused(format!("field `{}.{}`", td.name, f.name));
                }
            }
        }
        for f in &schema.functions {
            if f.params.iter().any(|(_, x)| mentions_def(x, def)) || mentions_def(&f.result, def) {
                return refused(format!("function `{}`", f.name));
            }
        }
        let own = FileId(self.decl_at(Item::Type(t)).expect("a declared type") as u32);
        for (file, _, r) in check::schema_refs(schema) {
            if file != own
                && matches!(r, check::Ref::Ctor(d, _) | check::Ref::Field { def: d, .. } if d == def)
            {
                return refused(self.shown_decl(file.0 as usize));
            }
        }
        Ok(())
    }

    fn drop_relation_allowed(&self, run: &Run, r: u64) -> Res<()> {
        let built = self.schema_now(run)?;
        let schema = built.schema();
        let Some(rel) = schema.ids().relation_of(crate::ids::CatRelId::from_raw(r)) else {
            return broken("a dropped relation variable the schema does not hold");
        };
        let name = &self.relations[&r].name;
        let own = schema.relation(rel).elem_def;
        for (j, td) in schema.types.iter().enumerate() {
            if j == own.0 as usize {
                continue;
            }
            for f in td
                .header()
                .iter()
                .chain(td.ctors().iter().flat_map(|c| c.fields.iter()))
            {
                if mentions_rel(&f.ty, rel) {
                    return refuse(format!(
                        "`drop relation {name}` is refused while field `{}.{}` references it",
                        td.name, f.name
                    ));
                }
            }
        }
        let file = FileId(
            self.decl_at(Item::Relation(r))
                .expect("a declared relation") as u32,
        );
        for (f, _, x) in check::schema_refs(schema) {
            if f != file && matches!(x, check::Ref::Relation(y) if y == rel) {
                return refuse(format!(
                    "`drop relation {name}` is refused while {} names it",
                    self.shown_decl(f.0 as usize)
                ));
            }
        }
        Ok(())
    }

    /// Rename object `(space, id)` to `to`, refusing a name taken and, as the operator reads
    /// them, declarations that do not build.
    pub(super) fn rename(
        &mut self,
        run: &Run,
        (space, id): Obj,
        to: &ast::Ident,
    ) -> Res<(Resolved, Vec<Clause>)> {
        let (at, to) = (to.span, to.text.clone());
        let resolved = match space {
            Space::Shared if self.types.contains_key(&id) => {
                let module = self
                    .module_of((space, id))
                    .expect("a declared type")
                    .to_string();
                if self.type_named(&module, &to).is_some() {
                    return refuse_at(
                        at,
                        format!("module `{module}` already declares type `{to}`"),
                    );
                }
                self.schema_now(run)?;
                let ty = self.types.get_mut(&id).expect("the type");
                ty.name = to.clone();
                if let TypeBody::Record { ctor } | TypeBody::New { ctor, .. } = ty.body {
                    self.ctors.get_mut(&ctor).expect("the constructor").name = to.clone();
                }
                Resolved::RenameType { sum: id, name: to }
            }
            Space::Shared if self.relations.contains_key(&id) => {
                let module = self
                    .module_of((space, id))
                    .expect("a declared relation")
                    .to_string();
                self.schema_now(run)?;
                let taken = self.decls.iter().any(|d| {
                    d.module == module
                        && match d.item {
                            Item::Relation(r) => self.relations[&r].name == to,
                            Item::Fn(f) | Item::Aggregate(f) => self.functions[&f].name == to,
                            Item::Constraint(c) | Item::Check(c) => self.denials[&c].name == to,
                            _ => false,
                        }
                });
                if taken {
                    return refuse_at(at, format!("module `{module}` already declares `{to}`"));
                }
                self.relations.get_mut(&id).expect("the relation").name = to.clone();
                Resolved::RenameRelation {
                    relation: id,
                    name: to,
                }
            }
            Space::Shared => {
                let ty = self.ctors[&id].ty;
                if self.ctor_named(ty, &to).is_some() {
                    return refuse_at(
                        at,
                        format!(
                            "`{to}` is already a constructor of `{}`",
                            self.types[&ty].name
                        ),
                    );
                }
                for r in self.relations_of(ty) {
                    if self
                        .sets_of(r)
                        .into_iter()
                        .any(|s| self.sets[&s].name == to)
                    {
                        return refuse_at(
                            at,
                            format!(
                                "`{to}` is already a named set of `{}`",
                                self.relations[&r].name
                            ),
                        );
                    }
                }
                self.ctors.get_mut(&id).expect("the constructor").name = to.clone();
                Resolved::RenameConstructor { ctor: id, name: to }
            }
            Space::Set => {
                self.schema_now(run)?;
                let r = self.sets[&id].relation;
                let clash = self
                    .sets_of(r)
                    .into_iter()
                    .any(|s| self.sets[&s].name == to)
                    || self
                        .relation_type(r)
                        .is_some_and(|t| self.ctor_named(t, &to).is_some());
                if clash {
                    return refuse_at(
                        at,
                        format!(
                            "`{}` already has a constructor or named set `{to}`",
                            self.relations[&r].name
                        ),
                    );
                }
                self.sets.get_mut(&id).expect("the set").name = to.clone();
                Resolved::RenameSet { set: id, name: to }
            }
            Space::Field => {
                let owner = self.fields[&id].owner;
                let t = self.field_type(id).expect("a field's type");
                let header = self.types.contains_key(&owner);
                let clash = self.fields_of(t).into_iter().any(|f| {
                    let x = &self.fields[&f];
                    x.name == to
                        && (header || self.types.contains_key(&x.owner) || x.owner == owner)
                });
                if clash {
                    let ctor = (!header).then_some(owner);
                    return refuse_at(
                        at,
                        format!("`{}` already has a field `{to}`", self.shown_part(t, ctor)),
                    );
                }
                self.schema_now(run)?;
                self.fields.get_mut(&id).expect("the field").name = to.clone();
                Resolved::RenameField {
                    field: id,
                    name: to,
                }
            }
            _ => return broken("a rename of this kind of object"),
        };
        Ok((resolved, Vec::new()))
    }

    pub(super) fn stage(
        &mut self,
        run: &Run,
        key: StateKey,
        to: Stage,
        verb: &str,
        shown: &str,
    ) -> Res<(Resolved, Vec<Clause>)> {
        let held = match key {
            StateKey::Relation(r, c) => self.states.get(&(r, c)),
            StateKey::Value(c) => self.value_states.get(&c),
        };
        let current = held.map_or(Stage::Enabled, |s| s.stage);
        match (verb, current) {
            ("enable", Stage::Enabled) => return refuse(format!("`{shown}` is already enabled")),
            ("enable", _) | ("disable" | "deprecate", Stage::Enabled) => {}
            ("disable", s) => {
                return refuse(format!(
                    "`disable` moves an enabled constructor back to declared; `{shown}` is {}",
                    s.name()
                ));
            }
            (_, s) => {
                return refuse(format!(
                    "`{verb}` applies only to an enabled constructor; `{shown}` is {}",
                    s.name()
                ));
            }
        }
        if to != Stage::Enabled
            && let StateKey::Relation(r, c) = key
        {
            let built = self.schema_now(run)?;
            let schema = built.schema();
            let rel = schema
                .ids()
                .relation_of(crate::ids::CatRelId::from_raw(r))
                .expect("a relation variable the model holds");
            let def = schema.relation(rel).elem_def;
            let k = schema
                .ids()
                .ctor_of(crate::ids::CatCtorId::from_raw(c))
                .expect("a constructor the model holds")
                .index();
            if let Some(f) = crate::evolve::retag_into(schema, def, k) {
                return refuse(format!(
                    "`{verb} {shown}` is refused: the delete action of field `{f}` retags into it"
                ));
            }
        }
        let state = State {
            stage: to,
            since: self.version,
        };
        let target = match key {
            StateKey::Relation(r, c) => {
                self.states.insert((r, c), state);
                Target::InRelation {
                    relation: r,
                    ctor: c,
                }
            }
            StateKey::Value(c) => {
                self.value_states.insert(c, state);
                Target::OfType { ctor: c }
            }
        };
        let resolved = match verb {
            "enable" => Resolved::Enable { target },
            "disable" => Resolved::Disable { target },
            _ => Resolved::Deprecate { target },
        };
        Ok((resolved, Vec::new()))
    }
}
