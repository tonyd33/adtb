//! Rust items for the types of a schema: declared types, relation markers and the anonymous
//! records and sums of signatures, each a host value.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write;

use adtb_lang::schema::{FieldDef, Schema, TypeDefKind};
use adtb_lang::textual::key_field_ty;
use adtb_lang::types::{Base, RelId, Ty, TypeId};

use crate::Error;
use crate::names::{camel, ident};

#[cfg(test)]
#[path = "emit_tests.rs"]
mod tests;

const HOST: &str = "::adtb::typed::HostValue";
const TYPED: &str = "::adtb::typed";
const DERIVES: &str = "#[derive(Clone, Debug, PartialEq)]";

/// Names a generated item may not take: those of the Rust prelude that generated code names.
const RESERVED: &[&str] = &[
    "Option", "Some", "None", "Result", "Ok", "Err", "String", "Vec", "Box", "Self",
];

/// The items of one generated module.
pub(crate) struct Module<'s> {
    schema: &'s Schema,
    pub(crate) out: String,
    wanted: VecDeque<Wanted>,
    types: BTreeSet<TypeId>,
    tags: BTreeSet<TypeId>,
    relations: BTreeSet<RelId>,
    /// Each Rust name an item of the module or of the module it re-exports takes, with what
    /// took it.
    names: BTreeMap<String, String>,
}

#[derive(Clone, Copy)]
enum Wanted {
    Type(TypeId),
    /// The tag type of a declared sum.
    Tag(TypeId),
    Relation(RelId),
}

/// A field of a generated struct or variant: its language name, Rust name and host type.
struct Field {
    name: String,
    rust: String,
    ty: String,
}

impl<'s> Module<'s> {
    pub(crate) fn new(schema: &'s Schema) -> Module<'s> {
        Module {
            schema,
            out: String::new(),
            wanted: VecDeque::new(),
            types: BTreeSet::new(),
            tags: BTreeSet::new(),
            relations: BTreeSet::new(),
            names: BTreeMap::new(),
        }
    }

    /// Returns a module over `schema` that re-exports the items of `prior` and emits only what
    /// `prior` lacks.
    pub(crate) fn after(schema: &'s Schema, prior: &Module<'_>) -> Module<'s> {
        Module {
            types: prior.types.clone(),
            tags: prior.tags.clone(),
            relations: prior.relations.clone(),
            names: prior.names.clone(),
            ..Module::new(schema)
        }
    }

    /// Emit every type and relation variable of the user modules.
    pub(crate) fn user_items(&mut self) -> Result<(), Error> {
        let schema = self.schema;
        for (i, t) in schema.types.iter().enumerate() {
            if !schema.module(t.module).engine {
                self.want(Wanted::Type(TypeId(i as u32)));
            }
        }
        for (i, r) in schema.relations.iter().enumerate() {
            if !schema.module(r.module).engine {
                self.want(Wanted::Relation(RelId(i as u32)));
            }
        }
        self.drain()
    }

    /// Take Rust name `name` for `what`, refusing one taken already.
    pub(crate) fn declare(&mut self, name: &str, what: &str) -> Result<(), Error> {
        if RESERVED.contains(&name) {
            return Err(Error::new(format!(
                "{what} takes the name `{name}`, which generated code needs for Rust's own"
            )));
        }
        if let Some(prior) = self.names.insert(name.to_owned(), what.to_owned()) {
            return Err(Error::new(format!(
                "{what} and {prior} both take the Rust name `{name}`"
            )));
        }
        Ok(())
    }

    /// Returns the host type of `ty`, naming an anonymous record or sum `anon` and a type
    /// parameter by `params`, and emits every item it needs.
    pub(crate) fn host(&mut self, ty: &Ty, anon: &str, params: &[String]) -> Result<String, Error> {
        let t = self.host_ty(ty, anon, params)?;
        self.drain()?;
        Ok(t)
    }

    fn want(&mut self, w: Wanted) {
        let fresh = match w {
            Wanted::Type(t) => self.types.insert(t),
            Wanted::Tag(t) => self.tags.insert(t),
            Wanted::Relation(r) => self.relations.insert(r),
        };
        if fresh {
            self.wanted.push_back(w);
        }
    }

    fn drain(&mut self) -> Result<(), Error> {
        while let Some(w) = self.wanted.pop_front() {
            match w {
                Wanted::Type(t) => self.declared(t)?,
                Wanted::Tag(t) => self.tag(t)?,
                Wanted::Relation(r) => self.relation(r)?,
            }
        }
        Ok(())
    }

    fn host_ty(&mut self, ty: &Ty, anon: &str, params: &[String]) -> Result<String, Error> {
        Ok(match ty {
            Ty::Base(b) => base(*b),
            Ty::Data { def, args, .. } => {
                let d = self.schema.ty(*def);
                let prelude = d.module == adtb_lang::schema::PRELUDE;
                match (prelude, d.name.as_str(), args.as_slice()) {
                    (true, "Bool", []) => "bool".into(),
                    (true, "Uuid", []) => format!("{TYPED}::Uuid"),
                    (true, "Option", [a]) => format!("Option<{}>", self.host_ty(a, anon, params)?),
                    _ => {
                        self.want(Wanted::Type(*def));
                        let mut out = d.name.clone();
                        if !args.is_empty() {
                            let mut hs = Vec::new();
                            for (i, a) in args.iter().enumerate() {
                                hs.push(self.host_ty(a, &format!("{anon}{i}"), params)?);
                            }
                            write!(out, "<{}>", hs.join(", ")).expect("a String takes writes");
                        }
                        out
                    }
                }
            }
            Ty::Param(i) => params
                .get(*i as usize)
                .cloned()
                .ok_or_else(|| Error::new(format!("type parameter {i} is not in scope")))?,
            Ty::Key { rel, .. } => {
                self.want(Wanted::Relation(*rel));
                format!("::adtb::Key<{}>", self.marker(*rel))
            }
            Ty::Set(t) | Ty::NonEmpty(t) => format!("Vec<{}>", self.host_ty(t, anon, params)?),
            Ty::Record(fs) if fs.is_empty() => "()".into(),
            Ty::Unit => "()".into(),
            Ty::Record(fs) => {
                self.anon_record(anon, fs, params)?;
                anon.to_owned()
            }
            Ty::Sum { ctors, .. } => {
                self.anon_sum(anon, ctors, params)?;
                anon.to_owned()
            }
            Ty::Tag(t) => match &**t {
                Ty::Data { def, .. } => {
                    self.want(Wanted::Tag(*def));
                    format!("{}Tag", self.schema.ty(*def).name)
                }
                Ty::Sum { ctors, .. } => {
                    let names: Vec<(String, Vec<Field>)> =
                        ctors.iter().map(|(c, _)| (c.clone(), Vec::new())).collect();
                    self.declare(anon, &format!("the anonymous tag type `{anon}`"))?;
                    enum_items(&mut self.out, anon, "", &names);
                    anon.to_owned()
                }
                _ => return Err(self.no_host(ty)),
            },
            Ty::Group(..) | Ty::Fn(..) | Ty::Change => {
                return Err(self.no_host(ty));
            }
        })
    }

    fn no_host(&self, ty: &Ty) -> Error {
        Error::new(format!("`{}` has no host type", self.schema.show_ty(ty)))
    }

    /// Emit the tag type of declared sum `t`: one variant per constructor, in declaration order.
    fn tag(&mut self, t: TypeId) -> Result<(), Error> {
        let d = self.schema.ty(t);
        let name = format!("{}Tag", d.name);
        self.declare(&name, &format!("the tag type of `{}`", d.name))?;
        let names: Vec<(String, Vec<Field>)> = d
            .ctors()
            .iter()
            .map(|c| (c.name.clone(), Vec::new()))
            .collect();
        enum_items(&mut self.out, &name, "", &names);
        Ok(())
    }

    /// Returns the Rust name of relation `rel`'s marker.
    fn marker(&self, rel: RelId) -> String {
        camel(&self.schema.relation(rel).name)
    }

    /// Returns the fields `fs` with their host types, naming an anonymous one after `owner`.
    fn fields<'f>(
        &mut self,
        owner: &str,
        fs: impl IntoIterator<Item = (&'f str, &'f Ty)>,
        params: &[String],
    ) -> Result<Vec<Field>, Error> {
        let mut out = Vec::new();
        for (name, ty) in fs {
            let ty = self.host_ty(ty, &format!("{owner}{}", camel(name)), params)?;
            out.push(Field {
                name: name.to_owned(),
                rust: ident(name),
                ty,
            });
        }
        Ok(out)
    }

    fn anon_record(
        &mut self,
        name: &str,
        fs: &[(String, Ty)],
        params: &[String],
    ) -> Result<(), Error> {
        self.declare(name, &format!("the anonymous record `{name}`"))?;
        let fields = self.fields(name, fs.iter().map(|(n, t)| (n.as_str(), t)), params)?;
        let out = &mut self.out;
        writeln!(out, "{DERIVES}\npub struct {name} {{").expect("a String takes writes");
        struct_fields(out, &fields);
        out.push_str("}\n\n");
        let to = format!("{TYPED}::of_record({})", values(&fields, "&self."));
        let from = format!(
            "let mut f = {TYPED}::record_fields(v)?;\n        {}",
            reads(&fields, name)
        );
        host_impl(out, name, "", &to, &from);
        Ok(())
    }

    fn anon_sum(
        &mut self,
        name: &str,
        ctors: &[(String, Vec<(String, Ty)>)],
        params: &[String],
    ) -> Result<(), Error> {
        self.declare(name, &format!("the anonymous sum `{name}`"))?;
        let mut variants = Vec::new();
        for (c, fs) in ctors {
            let owner = format!("{name}{c}");
            let fields = self.fields(&owner, fs.iter().map(|(n, t)| (n.as_str(), t)), params)?;
            variants.push((c.clone(), fields));
        }
        enum_items(&mut self.out, name, "", &variants);
        Ok(())
    }

    /// Emit declared type `t`.
    fn declared(&mut self, t: TypeId) -> Result<(), Error> {
        let schema = self.schema;
        let d = schema.ty(t);
        self.declare(&d.name, &format!("type `{}`", d.name))?;
        let params: Vec<String> = d.params.iter().map(|p| camel(p)).collect();
        let generics = if params.is_empty() {
            String::new()
        } else {
            format!("<{}>", params.join(", "))
        };
        let bounds = if params.is_empty() {
            String::new()
        } else {
            let bs: Vec<String> = params.iter().map(|p| format!("{p}: {HOST}")).collect();
            format!("<{}>", bs.join(", "))
        };
        let name = &d.name;
        let no_id = |what: String| Error::new(format!("{what} has no catalog id"));
        let id = schema
            .ids
            .types
            .get(&t)
            .ok_or_else(|| no_id(format!("type `{name}`")))?;
        let ctor_ids = (0..d.ctors().len().max(1))
            .map(|c| {
                let id = schema.ids.ctors.get(&(t, c as u32));
                id.map(u64::to_string).ok_or_else(|| {
                    let ctor = d.ctors().get(c).map_or(name.as_str(), |c| &c.name);
                    no_id(format!("constructor `{name}.{ctor}`"))
                })
            })
            .collect::<Result<Vec<String>, Error>>()?;
        match &d.kind {
            TypeDefKind::Newtype { base, .. } => {
                let rep = self.host_ty(base, &format!("{name}Value"), &params)?;
                writeln!(
                    self.out,
                    "{DERIVES}\npub struct {name}{generics}(pub {rep});\n"
                )
                .expect("a String takes writes");
                let to = format!("{HOST}::to_value(&self.0)");
                let from = format!("Some({name}({HOST}::from_value(v)?))");
                host_impl(
                    &mut self.out,
                    &format!("{name}{generics}"),
                    &bounds,
                    &to,
                    &from,
                );
            }
            TypeDefKind::Sum {
                header,
                ctors,
                record: true,
            } => {
                let fs = all_fields(header, &ctors[0].fields);
                let fields = self.fields(name, fs, &params)?;
                let out = &mut self.out;
                writeln!(out, "{DERIVES}\npub struct {name}{generics} {{")
                    .expect("a String takes writes");
                struct_fields(out, &fields);
                out.push_str("}\n\n");
                let to = format!("{TYPED}::of_ctor(0, {})", values(&fields, "&self."));
                let from = if fields.is_empty() {
                    format!("{TYPED}::ctor_fields(v, 0).map(|_| {name} {{}})")
                } else {
                    format!(
                        "let mut f = {TYPED}::ctor_fields(v, 0)?;\n        {}",
                        reads(&fields, name)
                    )
                };
                host_impl(out, &format!("{name}{generics}"), &bounds, &to, &from);
            }
            TypeDefKind::Sum { header, ctors, .. } => {
                let mut variants = Vec::new();
                for c in ctors {
                    let owner = format!("{name}{}", c.name);
                    let fields = self.fields(&owner, all_fields(header, &c.fields), &params)?;
                    variants.push((c.name.clone(), fields));
                }
                enum_items(
                    &mut self.out,
                    &format!("{name}{generics}"),
                    &bounds,
                    &variants,
                );
            }
        }
        writeln!(
            self.out,
            "impl{generics} {name}{generics} {{\n    pub const TYPE_ID: u64 = {id};\n    \
             pub const CTOR_IDS: [u64; {}] = [{}];\n}}\n",
            ctor_ids.len(),
            ctor_ids.join(", ")
        )
        .expect("a String takes writes");
        Ok(())
    }

    /// Emit the marker of relation variable `rel` and its constructor relations.
    fn relation(&mut self, rel: RelId) -> Result<(), Error> {
        let schema = self.schema;
        let r = schema.relation(rel);
        let marker = self.marker(rel);
        self.declare(&marker, &format!("relation `{}`", r.name))?;
        let module = &schema.module(r.module).name;
        let path = format!("{module}::{}", r.name);
        let key = match r.key.as_slice() {
            [k] => self.host_ty(&key_field_ty(schema, rel, k), &format!("{marker}Key"), &[])?,
            ks => {
                let mut fs: Vec<(String, Ty)> = ks
                    .iter()
                    .map(|k| (k.clone(), key_field_ty(schema, rel, k)))
                    .collect();
                fs.sort_by(|a, b| a.0.cmp(&b.0));
                self.host_ty(&Ty::Record(fs), &format!("{marker}Key"), &[])?
            }
        };
        let id = schema
            .ids
            .relations
            .get(&rel)
            .ok_or_else(|| Error::new(format!("relation `{}` has no catalog id", r.name)))?;
        let out = &mut self.out;
        writeln!(
            out,
            "#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub struct {marker};\n\n\
             impl {TYPED}::Relation for {marker} {{\n    type KeyValue = {key};\n    \
             const NAME: &'static str = \"{path}\";\n    const ID: u64 = {id};\n}}\n"
        )
        .expect("a String takes writes");
        let elem = schema.ty(r.elem_def);
        if matches!(elem.kind, TypeDefKind::Sum { record: true, .. }) {
            ctor_relation_impl(out, &marker, &path);
            return Ok(());
        }
        writeln!(out, "pub mod {} {{", ident(&r.name)).expect("a String takes writes");
        for c in elem.ctors() {
            ctor_relation(out, &c.name, &format!("{path}.{}", c.name));
        }
        out.push_str("}\n\n");
        Ok(())
    }
}

fn base(b: Base) -> String {
    match b {
        Base::I8 => "i8".into(),
        Base::I16 => "i16".into(),
        Base::I32 => "i32".into(),
        Base::I64 => "i64".into(),
        Base::F32 => "f32".into(),
        Base::F64 => "f64".into(),
        Base::Text => "String".into(),
        Base::Decimal(s) => format!("{TYPED}::Decimal<{s}>"),
        other => format!("{TYPED}::{other:?}"),
    }
}

fn all_fields<'f>(
    header: &'f [FieldDef],
    own: &'f [FieldDef],
) -> impl Iterator<Item = (&'f str, &'f Ty)> {
    header.iter().chain(own).map(|f| (f.name.as_str(), &f.ty))
}

fn struct_fields(out: &mut String, fields: &[Field]) {
    for f in fields {
        writeln!(out, "    pub {}: {},", f.rust, f.ty).expect("a String takes writes");
    }
}

/// Returns the `vec![...]` of each field's name and value, read from `at` followed by its name,
/// or from the bindings of [`binds`] when `at` is empty.
fn values(fields: &[Field], at: &str) -> String {
    if fields.is_empty() {
        return "Vec::new()".into();
    }
    let vs: Vec<String> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let read = if at.is_empty() {
                format!("x{i}")
            } else {
                format!("{at}{}", f.rust)
            };
            format!("(\"{}\", {HOST}::to_value({read}))", f.name)
        })
        .collect();
    format!("vec![{}]", vs.join(", "))
}

/// Returns the struct pattern fields that bind field `i` to `xi`: a unit struct may hold a
/// field's own name.
fn binds(fields: &[Field]) -> Vec<String> {
    fields
        .iter()
        .enumerate()
        .map(|(i, f)| format!("{}: x{i}", f.rust))
        .collect()
}

/// Returns the expression that reads each of `fields` from the iterator `f`, in order, into
/// `built`, a struct or variant path.
fn reads(fields: &[Field], built: &str) -> String {
    let nexts: Vec<String> = fields
        .iter()
        .map(|_| format!("{TYPED}::next(&mut f),"))
        .collect();
    let somes: Vec<String> = (0..fields.len()).map(|i| format!("Some(x{i}),")).collect();
    let binds = binds(fields);
    format!(
        "match ({}) {{\n            ({}) => Some({built} {{ {} }}),\n            _ => None,\n        }}",
        nexts.join(" "),
        somes.join(" "),
        binds.join(", ")
    )
}

fn host_impl(out: &mut String, ty: &str, bounds: &str, to: &str, from: &str) {
    writeln!(
        out,
        "impl{bounds} {HOST} for {ty} {{\n    fn to_value(&self) -> ::adtb::Value {{\n        \
         {to}\n    }}\n\n    fn from_value(v: &::adtb::Value) -> Option<Self> {{\n        \
         {from}\n    }}\n}}\n"
    )
    .expect("a String takes writes");
}

/// Emit enum `ty` of `variants`, each constructor in index order.
fn enum_items(out: &mut String, ty: &str, bounds: &str, variants: &[(String, Vec<Field>)]) {
    let name = ty.split('<').next().unwrap_or(ty);
    writeln!(out, "{DERIVES}\npub enum {ty} {{").expect("a String takes writes");
    for (c, fields) in variants {
        if fields.is_empty() {
            writeln!(out, "    {c},").expect("a String takes writes");
        } else {
            writeln!(out, "    {c} {{").expect("a String takes writes");
            for f in fields {
                writeln!(out, "        {}: {},", f.rust, f.ty).expect("a String takes writes");
            }
            out.push_str("    },\n");
        }
    }
    out.push_str("}\n\n");
    let mut to = String::from("match self {");
    let mut from = String::new();
    let mut readers = String::new();
    let any = variants.iter().any(|(_, fs)| !fs.is_empty());
    let parts = if any { "(c, mut f)" } else { "(c, _)" };
    write!(
        from,
        "let {parts} = {TYPED}::ctor_parts(v)?;\n        match c {{"
    )
    .expect("a String takes writes");
    for (i, (c, fields)) in variants.iter().enumerate() {
        let pat = if fields.is_empty() {
            format!("{name}::{c}")
        } else {
            format!("{name}::{c} {{ {} }}", binds(fields).join(", "))
        };
        write!(
            to,
            "\n            {pat} => {TYPED}::of_ctor({i}, {}),",
            values(fields, "")
        )
        .expect("a String takes writes");
        if fields.is_empty() {
            write!(from, "\n            {i} => Some({pat}),").expect("a String takes writes");
            continue;
        }
        write!(from, "\n            {i} => Self::read_{i}(&mut f),")
            .expect("a String takes writes");
        write!(
            readers,
            "\n    fn read_{i}<'v>(f: &mut impl Iterator<Item = &'v ::adtb::Value>) -> Option<Self> {{\n        \
             let mut f = f;\n        {}\n    }}\n",
            reads(fields, &format!("{name}::{c}")).replace("\n", "\n    ")
        )
        .expect("a String takes writes");
    }
    to.push_str("\n        }");
    from.push_str("\n            _ => None,\n        }");
    host_impl(out, ty, bounds, &to, &from);
    if !readers.is_empty() {
        writeln!(out, "impl{bounds} {ty} {{{readers}}}\n").expect("a String takes writes");
    }
}

fn ctor_relation(out: &mut String, name: &str, target: &str) {
    writeln!(
        out,
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub struct {name};\n"
    )
    .expect("a String takes writes");
    ctor_relation_impl(out, name, target);
}

fn ctor_relation_impl(out: &mut String, name: &str, target: &str) {
    writeln!(
        out,
        "impl ::adtb::CtorRelation for {name} {{\n    fn target(&self) -> &str {{\n        \
         \"{target}\"\n    }}\n}}\n"
    )
    .expect("a String takes writes");
}
