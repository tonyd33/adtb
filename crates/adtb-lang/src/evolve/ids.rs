use super::*;
use crate::ids::{
    KeyKind, Object, ObjectKey, Owner, Path, PathStep, Replacement, Slot, assign, objects,
};

impl Source {
    /// The text of a span of file `file`, its tokens joined by single spaces.
    fn span_text(&self, file: FileId, span: Span) -> String {
        let text = self.file_text(file);
        tokens(&text[span.range()])
            .into_iter()
            .map(|(_, r)| {
                text[span.start as usize + r.start..span.start as usize + r.end].to_string()
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The built schema and every object of its user modules.
    fn objects(&self) -> Option<(Arc<Schema>, Vec<Object>)> {
        let schema = self.built().ok()?;
        let files = self.files().ok()?;
        let out = objects(&schema, &files, &|f, s| self.span_text(f, s));
        Some((schema, out))
    }

    /// Gives every object without an id the next id of its space, in creation order, and forgets the
    /// ids of objects that are gone; ids are never reused.
    pub(super) fn reconcile_ids(&mut self) {
        let replaced = std::mem::take(&mut self.replaced);
        let Some((schema, objs)) = self.objects() else {
            return;
        };
        let replaced = resolved(&schema, &replaced);
        self.ids = assign(&objs, &self.ids, &mut self.next, &replaced)
            .into_iter()
            .collect();
    }

    /// Record that the operator being applied replaces `replaced`, read when its objects take
    /// ids.
    pub(super) fn replacing(&mut self, replaced: Vec<Replaced>) {
        self.replaced = replaced;
    }

    /// Returns the path of field `f` of type `t` of module `m`, of constructor `ctor` or the
    /// header.
    pub(super) fn field_path(&self, m: &str, t: &str, ctor: Option<&str>, f: &str) -> Option<Path> {
        self.field_key_id(m, t, ctor, f)
            .map(|id| vec![PathStep::Field(id)])
    }

    /// Write the source's catalog ids and write states into a schema it built. An object the log
    /// has no id for takes the id [`Source::reconcile_ids`] gives it once the operator is recorded.
    pub(super) fn annotate(&self, schema: &mut Schema) {
        if let Ok(files) = self.files() {
            let objs = objects(schema, &files, &|f, s| self.span_text(f, s));
            let replaced = resolved(schema, &self.replaced);
            let ids = assign(&objs, &self.ids, &mut self.next.clone(), &replaced);
            for o in &objs {
                schema.ids.forget(o.slot());
            }
            for (o, (_, id)) in objs.iter().zip(ids) {
                schema.ids.set(o.slot(), id);
            }
        }
        let mut stages = Vec::new();
        for (i, r) in schema.relations.iter().enumerate() {
            if schema.module(r.module).engine {
                continue;
            }
            let m = schema.module(r.module).name.clone();
            for (c, ctor) in schema.ty(r.elem_def).ctors().iter().enumerate() {
                if let Some(s) = self
                    .states
                    .get(&(m.clone(), r.name.clone(), ctor.name.clone()))
                    && s.stage != Stage::Enabled
                {
                    stages.push(((RelId(i as u32), c as u32), s.stage));
                }
            }
        }
        let mut value_stages = Vec::new();
        for (i, t) in schema.types.iter().enumerate() {
            if schema.module(t.module).engine {
                continue;
            }
            let m = schema.module(t.module).name.clone();
            for (c, ctor) in t.ctors().iter().enumerate() {
                if let Some(s) =
                    self.value_states
                        .get(&(m.clone(), t.name.clone(), ctor.name.clone()))
                    && s.stage != Stage::Enabled
                {
                    value_stages.push(((TypeId(i as u32), c as u32), s.stage));
                }
            }
        }
        schema.stages.extend(stages);
        schema.value_stages.extend(value_stages);
    }

    /// Move the id of each object a rename or move operator renames or moves to its key after
    /// the operator, `before` the source the operator applied to.
    pub(super) fn rename_ids(&mut self, before: &Source, module: &str, op: &OperatorKind) {
        let q = |n: &QName| {
            n.module
                .as_ref()
                .map_or(module.to_string(), |m| m.text.clone())
        };
        let moves = match op {
            OperatorKind::RenameType(t, to) => {
                self.type_renamed(before, &q(t), &t.name.text, &to.text)
            }
            OperatorKind::RenameConstructor(p, to) => {
                let ty = self.type_owner(&q(&p.ty), &p.ty.name.text);
                vec![named(KeyKind::Ctor, ty, &p.ctor.text, &to.text)]
            }
            OperatorKind::RenameRelation(r, to) => {
                let m = Some(Owner::Module(q(r)));
                vec![named(KeyKind::Relation, m, &r.name.text, &to.text)]
            }
            OperatorKind::RenameSet(set, r, to) => {
                let rel = self.object_owner(&ObjectKey::named(
                    KeyKind::Relation,
                    Owner::Module(q(r)),
                    &r.name.text,
                ));
                vec![named(KeyKind::Set, rel, &set.text, &to.text)]
            }
            OperatorKind::RenameField(p, to) => {
                let m = q(&p.ty);
                let ctor = match &p.ctor {
                    Some(c) => Some(c.text.as_str()),
                    None if is_record(before, &m, &p.ty.name.text) => Some(p.ty.name.text.as_str()),
                    None => None,
                };
                let owner = self.field_owner(&m, &p.ty.name.text, ctor);
                vec![named(KeyKind::Field, owner, &p.field.text, &to.text)]
            }
            OperatorKind::MoveField { path, sel, .. } => {
                self.field_moved(before, &q(&path.ty), path, sel.as_ref())
            }
            OperatorKind::AddConstructor { path, .. } => {
                self.record_widened(before, &q(&path.ty), &path.ty.name.text)
            }
            _ => Vec::new(),
        };
        for (from, to) in moves.into_iter().flatten() {
            self.rekey(&from, to);
        }
    }

    /// Move the id at key `from` to key `to`.
    pub(super) fn rekey(&mut self, from: &ObjectKey, to: ObjectKey) {
        if let Some(id) = self.ids.remove(from) {
            self.ids.insert(to, id);
        }
    }

    /// Returns the object at key `key` as an owner.
    fn object_owner(&self, key: &ObjectKey) -> Option<Owner> {
        self.ids.get(key).map(|&id| Owner::Object(id))
    }

    /// Returns type `t` of module `m` as an owner.
    pub(super) fn type_owner(&self, m: &str, t: &str) -> Option<Owner> {
        field_owner(&self.ids, m, t, None)
    }

    /// Returns what holds a field of type `t` of module `m`: constructor `ctor`, else the header.
    pub(super) fn field_owner(&self, m: &str, t: &str, ctor: Option<&str>) -> Option<Owner> {
        field_owner(&self.ids, m, t, ctor)
    }

    /// Returns the id of field `f` of type `t` of module `m`, of constructor `ctor` or the header.
    pub(super) fn field_key_id(
        &self,
        m: &str,
        t: &str,
        ctor: Option<&str>,
        f: &str,
    ) -> Option<u64> {
        field_key_id(&self.ids, m, t, ctor, f)
    }

    /// Returns the id of constructor `c` of type `t` of module `m`.
    pub(super) fn ctor_key_id(&self, m: &str, t: &str, c: &str) -> Option<u64> {
        ctor_key_id(&self.ids, m, t, c)
    }

    fn type_renamed(&self, before: &Source, m: &str, from: &str, to: &str) -> Vec<Option<Moved>> {
        let module = Some(Owner::Module(m.to_string()));
        let mut out = vec![named(KeyKind::Type, module, from, to)];
        let named_after = before.built().ok().is_some_and(|schema| {
            schema
                .module_named(m)
                .and_then(|mid| schema.module(mid).types.get(from).copied())
                .is_some_and(|def| {
                    let td = schema.ty(def);
                    matches!(td.kind, TypeDefKind::Newtype { .. }) || td.ctor_named(from).is_some()
                })
        });
        if named_after {
            out.push(named(KeyKind::Ctor, self.type_owner(m, from), from, to));
        }
        out
    }

    fn field_moved(
        &self,
        before: &Source,
        m: &str,
        path: &ast::FieldPath,
        sel: Option<&CtorSel>,
    ) -> Vec<Option<Moved>> {
        let t = &path.ty.name.text;
        let f = &path.field.text;
        let (from, to) = match (&path.ctor, sel) {
            (Some(c), None) => (Some(c.text.clone()), None),
            (None, Some(sel)) => {
                let Some(first) = before.built().ok().and_then(|schema| {
                    let def = *schema.module(schema.module_named(m)?).types.get(t)?;
                    sel_idents(sel)
                        .iter()
                        .filter_map(|x| schema.ty(def).ctor_index(&x.text))
                        .min()
                        .map(|k| schema.ty(def).ctors()[k as usize].name.clone())
                }) else {
                    return Vec::new();
                };
                (None, Some(first))
            }
            _ => return Vec::new(),
        };
        vec![relocated(
            self.field_owner(m, t, from.as_deref()),
            self.field_owner(m, t, to.as_deref()),
            f,
        )]
    }

    /// Returns the moves of the key fields of record type `t` of module `m`, which an added
    /// constructor makes a sum, from its constructor to its header.
    fn record_widened(&self, before: &Source, m: &str, t: &str) -> Vec<Option<Moved>> {
        let Ok(schema) = before.built() else {
            return Vec::new();
        };
        let Some(def) = schema
            .module_named(m)
            .and_then(|mid| schema.module(mid).types.get(t).copied())
        else {
            return Vec::new();
        };
        if !matches!(schema.ty(def).kind, TypeDefKind::Sum { record: true, .. }) {
            return Vec::new();
        }
        let keys: BTreeSet<&String> = schema
            .relations
            .iter()
            .filter(|r| r.elem_def == def)
            .flat_map(|r| &r.key)
            .collect();
        keys.into_iter()
            .map(|k| relocated(self.field_owner(m, t, Some(t)), self.type_owner(m, t), k))
            .collect()
    }
}

/// Returns what holds a field of type `t` of module `m` among `ids`: constructor `ctor`, else
/// the header.
pub(super) fn field_owner(
    ids: &BTreeMap<ObjectKey, u64>,
    m: &str,
    t: &str,
    ctor: Option<&str>,
) -> Option<Owner> {
    let ty = ids.get(&ObjectKey::named(
        KeyKind::Type,
        Owner::Module(m.to_string()),
        t,
    ))?;
    let ty = Owner::Object(*ty);
    match ctor {
        Some(c) => ids
            .get(&ObjectKey::named(KeyKind::Ctor, ty, c))
            .map(|&id| Owner::Object(id)),
        None => Some(ty),
    }
}

/// Returns the id among `ids` of field `f` of type `t` of module `m`, of constructor `ctor` or
/// the header.
pub(super) fn field_key_id(
    ids: &BTreeMap<ObjectKey, u64>,
    m: &str,
    t: &str,
    ctor: Option<&str>,
    f: &str,
) -> Option<u64> {
    let owner = field_owner(ids, m, t, ctor)?;
    ids.get(&ObjectKey::named(KeyKind::Field, owner, f))
        .copied()
}

/// Returns the id among `ids` of constructor `c` of type `t` of module `m`.
pub(super) fn ctor_key_id(
    ids: &BTreeMap<ObjectKey, u64>,
    m: &str,
    t: &str,
    c: &str,
) -> Option<u64> {
    let ty = field_owner(ids, m, t, None)?;
    ids.get(&ObjectKey::named(KeyKind::Ctor, ty, c)).copied()
}

/// A field or a constructor of the schema an operator gives, by module, type, constructor and
/// name.
#[derive(Clone, Debug)]
pub(super) enum Held {
    Field(String, String, Option<String>, String),
    Ctor(String, String, String),
}

/// A path of ids an operator replaces, and what holds its values in the schema it gives.
#[derive(Clone, Debug)]
pub(super) struct Replaced {
    from: Path,
    to: Vec<Held>,
}

impl Replaced {
    pub(super) fn new(from: Path, to: Vec<Held>) -> Replaced {
        Replaced { from, to }
    }
}

/// Returns each of `replaced` whose every step after the operator `schema` names.
fn resolved(schema: &Schema, replaced: &[Replaced]) -> Vec<Replacement> {
    let def = |m: &str, t: &str| {
        let mid = schema.module_named(m)?;
        schema.module(mid).types.get(t).copied()
    };
    let slot = |h: &Held| match h {
        Held::Field(m, t, c, f) => {
            let td = schema.ty(def(m, t)?);
            let ctor = match c {
                Some(c) => Some(td.ctor_named(c)?),
                None => None,
            };
            td.field(ctor, f).map(Slot::Field)
        }
        Held::Ctor(m, t, c) => schema.ty(def(m, t)?).ctor_named(c).map(Slot::Ctor),
    };
    replaced
        .iter()
        .filter_map(|r| {
            let to = r.to.iter().map(slot).collect::<Option<Vec<Slot>>>()?;
            Some(Replacement::new(r.from.clone(), to))
        })
        .collect()
}

/// A key and the key it takes.
type Moved = (ObjectKey, ObjectKey);

/// Returns the move of the object of kind `kind` named `from` that `owner` holds to name `to`.
fn named(kind: KeyKind, owner: Option<Owner>, from: &str, to: &str) -> Option<Moved> {
    let from = ObjectKey::named(kind, owner?, from);
    let to = from.renamed(to);
    Some((from, to))
}

/// Returns the move of field `f` from owner `from` to owner `to`.
fn relocated(from: Option<Owner>, to: Option<Owner>, f: &str) -> Option<Moved> {
    let from = ObjectKey::named(KeyKind::Field, from?, f);
    let to = from.owned_by(to?);
    Some((from, to))
}

fn is_record(before: &Source, m: &str, t: &str) -> bool {
    before
        .find("type", m, t)
        .and_then(|i| before.parsed(i))
        .is_some_and(|d| {
            matches!(
                d,
                Decl::Type(ast::TypeDecl {
                    body: ast::TypeBody::Record(_),
                    ..
                })
            )
        })
}
