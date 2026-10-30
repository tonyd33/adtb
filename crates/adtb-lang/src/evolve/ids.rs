use super::*;
use crate::ids::{FIRST_LOG_ID, Object, objects};

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

    /// The built schema and every object of its user modules, each path made distinct.
    fn objects(&self) -> Option<(Arc<Schema>, Vec<Object>)> {
        let schema = self.built().ok()?;
        let files = self.files().ok()?;
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        let mut out = objects(&schema, &files, &|f, s| self.span_text(f, s));
        for o in &mut out {
            let n = seen.entry(o.path.clone()).or_default();
            if *n > 0 {
                o.path = format!("{}#{n}", o.path);
            }
            *n += 1;
        }
        Some((schema, out))
    }

    /// Gives every object without an id the next id of its space, in creation order, and forgets the
    /// ids of objects that are gone; ids are never reused.
    pub(super) fn reconcile_ids(&mut self) {
        let Some((_, objs)) = self.objects() else {
            return;
        };
        let mut ids = BTreeMap::new();
        for o in objs {
            let id = match self.ids.get(&o.path) {
                Some(&id) => id,
                None => {
                    let n = self.next.entry(o.space).or_insert(FIRST_LOG_ID);
                    *n += 1;
                    *n - 1
                }
            };
            ids.insert(o.path, id);
        }
        self.ids = ids;
    }

    /// Writes the source's catalog ids and write states into a schema it built.
    pub(super) fn annotate(&self, schema: &mut Schema) {
        if let Ok(files) = self.files() {
            let objs = objects(schema, &files, &|f, s| self.span_text(f, s));
            let mut seen: BTreeMap<String, usize> = BTreeMap::new();
            for o in objs {
                let n = seen.entry(o.path.clone()).or_default();
                let path = if *n > 0 {
                    format!("{}#{n}", o.path)
                } else {
                    o.path.clone()
                };
                *n += 1;
                if let Some(&id) = self.ids.get(&path) {
                    schema.ids.set(&o.slot, id);
                }
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

    /// Renames the paths of the ids a rename operator's object and its parts have.
    pub(super) fn rename_ids(&mut self, before: &Source, module: &str, op: &OperatorKind) {
        let q = |n: &QName| {
            n.module
                .as_ref()
                .map_or(module.to_string(), |m| m.text.clone())
        };
        match op {
            OperatorKind::RenameType(t, to) => {
                self.rename_type_ids(before, &q(t), &t.name.text, &to.text);
            }
            OperatorKind::RenameConstructor(p, to) => {
                self.rename_ctor_ids(before, &q(&p.ty), &p.ty.name.text, &p.ctor.text, &to.text);
            }
            OperatorKind::RenameRelation(r, to) => {
                let m = q(r);
                let (from, to) = (format!("{m}::{}", r.name.text), format!("{m}::{}", to.text));
                rewrite(&mut self.ids, &|k| {
                    if k == format!("relation {from}") {
                        return Some(format!("relation {to}"));
                    }
                    for (kind, sep) in [
                        ("index", " "),
                        ("default", " "),
                        ("computed", "."),
                        ("set", "."),
                    ] {
                        if let Some(rest) = k.strip_prefix(&format!("{kind} {from}{sep}")) {
                            return Some(format!("{kind} {to}{sep}{rest}"));
                        }
                    }
                    None
                });
            }
            OperatorKind::RenameSet(set, r, to) => {
                let rel = format!("{}::{}", q(r), r.name.text);
                let (from, to) = (set.text.clone(), to.text.clone());
                rewrite(&mut self.ids, &|k| {
                    if k == format!("set {rel}.{from}") {
                        return Some(format!("set {rel}.{to}"));
                    }
                    let rest = k.strip_prefix(&format!("index {rel} "))?;
                    let (head, scope) = rest.rsplit_once(" in ")?;
                    Some(format!(
                        "index {rel} {head} in {}",
                        replace_word(scope, &from, &to)
                    ))
                });
            }
            OperatorKind::RenameField(p, to) => {
                self.rename_field_ids(before, &q(&p.ty), p, &to.text);
            }
            OperatorKind::MoveField { path, sel, .. } => {
                self.move_field_ids(before, &q(&path.ty), path, sel.as_ref());
            }
            OperatorKind::AddConstructor { path, .. } => {
                self.add_ctor_ids(before, &q(&path.ty), &path.ty.name.text);
            }
            _ => {}
        }
    }

    fn rename_type_ids(&mut self, before: &Source, m: &str, from: &str, to: &str) {
        let record = is_record(before, m, from);
        rewrite(&mut self.ids, &|k| {
            if k == format!("type {m}::{from}") {
                return Some(format!("type {m}::{to}"));
            }
            for kind in ["ctor", "field"] {
                let p = format!("{kind} {m}::{from}.");
                if let Some(rest) = k.strip_prefix(&p) {
                    let rest = match rest.split_once('.') {
                        Some((c, f)) if record && c == from => format!("{to}.{f}"),
                        None if record && kind == "ctor" && rest == from => to.to_string(),
                        _ => rest.to_string(),
                    };
                    return Some(format!("{kind} {m}::{to}.{rest}"));
                }
            }
            None
        });
    }

    fn rename_ctor_ids(&mut self, before: &Source, m: &str, t: &str, from: &str, to: &str) {
        let rels = relation_paths(before, m, t);
        rewrite(&mut self.ids, &|k| {
            if k == format!("ctor {m}::{t}.{from}") {
                return Some(format!("ctor {m}::{t}.{to}"));
            }
            if let Some(f) = k.strip_prefix(&format!("field {m}::{t}.{from}.")) {
                return Some(format!("field {m}::{t}.{to}.{f}"));
            }
            for r in &rels {
                if let Some(rest) = k.strip_prefix(&format!("index {r} "))
                    && let Some((head, scope)) = rest.rsplit_once(" in ")
                {
                    return Some(format!(
                        "index {r} {head} in {}",
                        replace_word(scope, from, to)
                    ));
                }
                if let Some(rest) = k.strip_prefix(&format!("default {r} {from}.")) {
                    return Some(format!("default {r} {to}.{rest}"));
                }
            }
            None
        });
    }

    fn rename_field_ids(&mut self, before: &Source, m: &str, p: &ast::FieldPath, to: &str) {
        let t = &p.ty.name.text;
        let from = &p.field.text;
        let at = match &p.ctor {
            Some(c) => format!("{t}.{}.", c.text),
            None if is_record(before, m, t) => format!("{t}.{t}."),
            None => format!("{t}."),
        };
        let rels = relation_paths(before, m, t);
        rewrite(&mut self.ids, &|k| {
            if k == format!("field {m}::{at}{from}") {
                return Some(format!("field {m}::{at}{to}"));
            }
            for r in &rels {
                if let Some(rest) = k.strip_prefix(&format!("index {r} "))
                    && let Some((head, scope)) = rest.rsplit_once(" in ")
                {
                    return Some(format!(
                        "index {r} {} in {scope}",
                        replace_word(head, from, to)
                    ));
                }
                if let Some(rest) = k.strip_prefix(&format!("default {r} ")) {
                    return Some(format!("default {r} {}", replace_word(rest, from, to)));
                }
            }
            None
        });
    }

    fn move_field_ids(
        &mut self,
        before: &Source,
        m: &str,
        path: &ast::FieldPath,
        sel: Option<&CtorSel>,
    ) {
        let t = &path.ty.name.text;
        let f = &path.field.text;
        let (from, to) = match (&path.ctor, sel) {
            (Some(c), None) => (format!("{t}.{}.{f}", c.text), format!("{t}.{f}")),
            (None, Some(sel)) => {
                let Some(first) = before.built().ok().and_then(|schema| {
                    let def = *schema.module(schema.module_named(m)?).types.get(t)?;
                    sel_idents(sel)
                        .iter()
                        .filter_map(|x| schema.ty(def).ctor_index(&x.text))
                        .min()
                        .map(|k| schema.ty(def).ctors()[k as usize].name.clone())
                }) else {
                    return;
                };
                (format!("{t}.{f}"), format!("{t}.{first}.{f}"))
            }
            _ => return,
        };
        let (from, to) = (format!("field {m}::{from}"), format!("field {m}::{to}"));
        rewrite(&mut self.ids, &|k| (k == from).then(|| to.clone()));
    }

    fn add_ctor_ids(&mut self, before: &Source, m: &str, t: &str) {
        let Ok(schema) = before.built() else {
            return;
        };
        let Some(def) = schema
            .module_named(m)
            .and_then(|mid| schema.module(mid).types.get(t).copied())
        else {
            return;
        };
        if !matches!(schema.ty(def).kind, TypeDefKind::Sum { record: true, .. }) {
            return;
        }
        let keys: BTreeSet<&String> = schema
            .relations
            .iter()
            .filter(|r| r.elem_def == def)
            .flat_map(|r| &r.key)
            .collect();
        let field = format!("field {m}::{t}.{t}.");
        rewrite(&mut self.ids, &|k| {
            let f = k.strip_prefix(&field)?;
            keys.iter()
                .any(|x| *x == f)
                .then(|| format!("field {m}::{t}.{f}"))
        });
    }
}

/// Returns the paths of the relations `before` declares over type `t` of module `m`.
fn relation_paths(before: &Source, m: &str, t: &str) -> Vec<String> {
    let Ok(schema) = before.built() else {
        return Vec::new();
    };
    schema
        .relations
        .iter()
        .filter(|r| {
            r.elem_def.0 != u32::MAX
                && schema.ty(r.elem_def).name == t
                && schema.module(schema.ty(r.elem_def).module).name == m
        })
        .map(|r| format!("{}::{}", schema.module(r.module).name, r.name))
        .collect()
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

/// Rewrite every key of `ids` that `f` maps.
fn rewrite(ids: &mut BTreeMap<String, u64>, f: &dyn Fn(&str) -> Option<String>) {
    let old: Vec<(String, u64)> = ids.iter().map(|(k, v)| (k.clone(), *v)).collect();
    ids.clear();
    for (k, v) in old {
        ids.insert(f(&k).unwrap_or(k), v);
    }
}

/// `text` with every whole word `from` replaced by `to`.
fn replace_word(text: &str, from: &str, to: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        out.push_str(if word == from { to } else { word });
        word.clear();
    };
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            word.push(ch);
        } else {
            flush(&mut word, &mut out);
            out.push(ch);
        }
    }
    flush(&mut word, &mut out);
    out
}
