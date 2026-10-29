use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

use crate::ast;
use crate::parse::{self, Options};
use crate::span::{Diag, FileDiag, FileId, Span};
use crate::types::*;

/// The declarations of the prelude module.
pub const PRELUDE_SRC: &str = include_str!("../prelude.adt");

/// The engine's declarations of its `catalog` and `sys` modules.
pub const ENGINE_SRC: &str = include_str!("../catalog.adt");

/// The modules the engine declares, which no user file may name.
pub const ENGINE_MODULES: [&str; 2] = ["catalog", "sys"];

#[cfg(test)]
#[path = "../schema_tests.rs"]
mod tests;

mod build;
mod relation;
mod types;

pub use types::resolve_type;

#[derive(Clone, Debug)]
pub struct ModuleDef {
    pub name: String,
    /// Whether the engine declares the module: the prelude, `catalog` or `sys`.
    pub engine: bool,
    pub types: BTreeMap<String, TypeId>,
    pub values: BTreeMap<String, ValueDef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueDef {
    Relation(RelId),
    Function(FnId),
    Constraint(u32),
    Check(u32),
    Aggregate(u32),
}

#[derive(Clone, Debug)]
pub struct FieldDef {
    pub name: String,
    pub ty: Ty,
    pub on_delete: OnDelete,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OnDelete {
    Restrict,
    Cascade,
    Clear,
    Retag(u32, Option<ast::RecordExpr>),
}

#[derive(Clone, Debug)]
pub struct CtorDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeDefKind {
    /// A sum; a record is one with a single constructor named after the type.
    Sum {
        header: Vec<FieldDef>,
        ctors: Vec<CtorDef>,
        record: bool,
    },
    Newtype {
        base: Ty,
        domain: Option<Box<ast::Expr>>,
    },
}

#[derive(Clone, Debug)]
pub struct TypeDef {
    pub name: String,
    pub module: ModuleId,
    pub params: Vec<String>,
    pub ordered: bool,
    pub kind: TypeDefKind,
    pub file: FileId,
    pub span: Span,
}

impl TypeDef {
    pub fn header(&self) -> &[FieldDef] {
        match &self.kind {
            TypeDefKind::Sum { header, .. } => header,
            TypeDefKind::Newtype { .. } => &[],
        }
    }

    pub fn ctors(&self) -> &[CtorDef] {
        match &self.kind {
            TypeDefKind::Sum { ctors, .. } => ctors,
            TypeDefKind::Newtype { .. } => &[],
        }
    }

    pub fn ctor_index(&self, name: &str) -> Option<u32> {
        self.ctors()
            .iter()
            .position(|c| c.name == name)
            .map(|i| i as u32)
    }

    /// Returns the field `name` of constructor `ctor`, header fields included.
    pub fn field(&self, ctor: u32, name: &str) -> Option<&FieldDef> {
        self.header().iter().find(|f| f.name == name).or_else(|| {
            self.ctors()
                .get(ctor as usize)?
                .fields
                .iter()
                .find(|f| f.name == name)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Generator {
    Sequence,
    Uuidv7,
    Clock,
}

#[derive(Clone, Debug)]
pub struct IndexDef {
    pub fields: Vec<ast::Expr>,
    pub scope: Option<CtorSet>,
    pub interval: bool,
    pub unique: bool,
}

#[derive(Clone, Debug)]
pub enum DefaultValue {
    Generate(Generator),
    Expr(Box<ast::Expr>),
}

#[derive(Clone, Debug)]
pub struct DefaultDef {
    pub ctor: Option<u32>,
    pub field: String,
    pub value: DefaultValue,
}

#[derive(Clone, Debug)]
pub struct ComputedDef {
    pub name: String,
    pub ty: Ty,
    pub expr: ast::Expr,
}

#[derive(Clone, Debug)]
pub struct NamedSet {
    pub name: String,
    pub ctors: CtorSet,
}

#[derive(Clone, Debug)]
pub struct RelationDef {
    pub name: String,
    pub module: ModuleId,
    /// The element type, always a declared sum at concrete arguments.
    pub elem: Ty,
    pub elem_def: TypeId,
    pub key: Vec<String>,
    pub generate: Option<Generator>,
    pub indexes: Vec<IndexDef>,
    pub defaults: Vec<DefaultDef>,
    pub computed: Vec<ComputedDef>,
    pub sets: Vec<NamedSet>,
    pub volatile: bool,
    pub file: FileId,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FunctionDef {
    pub name: String,
    pub module: ModuleId,
    pub type_params: Vec<(String, Vec<Bound>)>,
    pub params: Vec<(String, Ty)>,
    pub result: Ty,
    pub body: ast::FnBody,
    pub stored: Option<ast::StoreDecl>,
    pub file: FileId,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ConstraintDef {
    pub name: String,
    pub module: ModuleId,
    pub query: ast::Expr,
    pub file: FileId,
}

#[derive(Clone, Debug)]
pub struct CheckDef {
    pub name: String,
    pub module: ModuleId,
    pub relation: RelId,
    pub ctors: Option<CtorSet>,
    pub predicate: ast::Expr,
    pub file: FileId,
}

#[derive(Clone, Debug)]
pub struct AggregateDef {
    pub name: String,
    pub module: ModuleId,
    pub decl: ast::AggregateDecl,
    pub params: Vec<(String, Ty)>,
    pub result: Ty,
    /// The state record, its fields sorted by name.
    pub state: Ty,
    pub file: FileId,
}

/// A type name as resolution finds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeName {
    Decl(TypeId),
    Base(Base),
    Decimal,
    Set,
    Group,
    Tag,
}

#[derive(Clone, Debug)]
pub struct Schema {
    pub modules: Vec<ModuleDef>,
    pub types: Vec<TypeDef>,
    pub relations: Vec<RelationDef>,
    pub functions: Vec<FunctionDef>,
    pub constraints: Vec<ConstraintDef>,
    pub checks: Vec<CheckDef>,
    pub aggregates: Vec<AggregateDef>,
    /// The `Principal` every module resolves: the schema's own, else the prelude's.
    pub principal: TypeId,
    pub ids: crate::ids::CatalogIds,
    /// The write state of each constructor relation that is not enabled.
    pub stages: BTreeMap<(RelId, u32), Stage>,
    /// The write state of each value-level constructor that is not enabled.
    pub value_stages: BTreeMap<(TypeId, u32), Stage>,
    /// The relation variables a later operator of the migration being applied adds, by module and
    /// name. Entry `i` is `RelId(u32::MAX - i)`, which only a `Key r` type holds and no
    /// `relations` entry has.
    pub ahead: Vec<(ModuleId, String)>,
    /// The relation variables, by module and name, that the migration being applied adds with
    /// their type: a reference type's selection of their constructors resolves when it ends.
    pub late_sels: Vec<(ModuleId, String)>,
}

/// The write state of a constructor in a relation variable, or of a value-level constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Declared,
    Enabled,
    Deprecated,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Declared => "declared",
            Stage::Enabled => "enabled",
            Stage::Deprecated => "deprecated",
        }
    }
}

pub const PRELUDE: ModuleId = ModuleId(0);

impl Schema {
    pub fn module(&self, id: ModuleId) -> &ModuleDef {
        &self.modules[id.0 as usize]
    }

    /// Returns the write state of constructor `c` in relation variable `r`.
    pub fn stage(&self, r: RelId, c: u32) -> Stage {
        self.stages.get(&(r, c)).copied().unwrap_or(Stage::Enabled)
    }

    /// Returns the write state of constructor `c` of `t` as a value stored in a field.
    pub fn value_stage(&self, t: TypeId, c: u32) -> Stage {
        self.value_stages
            .get(&(t, c))
            .copied()
            .unwrap_or(Stage::Enabled)
    }

    pub fn module_named(&self, name: &str) -> Option<ModuleId> {
        self.modules
            .iter()
            .position(|m| m.name == name)
            .map(|i| ModuleId(i as u32))
    }

    pub fn ty(&self, id: TypeId) -> &TypeDef {
        &self.types[id.0 as usize]
    }

    pub fn relation(&self, id: RelId) -> &RelationDef {
        self.relations.get(id.0 as usize).unwrap_or_else(|| {
            panic!("{id:?} is a declared relation: one a later operator adds has no definition")
        })
    }

    /// Returns whether a reference type's selection of the constructors of `id` resolves when the
    /// migration being applied ends.
    pub fn selects_late(&self, id: RelId) -> bool {
        let named = match self.ahead_relation(id) {
            Some((m, name)) => (*m, name.as_str()),
            None => {
                let r = self.relation(id);
                (r.module, r.name.as_str())
            }
        };
        self.late_sels
            .iter()
            .any(|(m, n)| (*m, n.as_str()) == named)
    }

    /// Returns the module and name of `id` when a later operator of the migration adds it.
    pub fn ahead_relation(&self, id: RelId) -> Option<&(ModuleId, String)> {
        self.ahead.get((u32::MAX - id.0) as usize)
    }

    pub fn function(&self, id: FnId) -> &FunctionDef {
        &self.functions[id.0 as usize]
    }

    pub fn prelude_type(&self, name: &str) -> TypeId {
        self.module(PRELUDE).types[name]
    }

    /// Resolve unqualified `Principal` from the root, and other type names from the module, prelude or built-ins.
    pub fn lookup_type(&self, module: ModuleId, name: &ast::QName) -> Result<TypeName, String> {
        let text = name.name.text.as_str();
        if let Some(m) = &name.module {
            let mid = self
                .module_named(&m.text)
                .ok_or_else(|| format!("no module `{}`", m.text))?;
            return self
                .module(mid)
                .types
                .get(text)
                .map(|&t| TypeName::Decl(t))
                .ok_or_else(|| format!("module `{}` declares no type `{text}`", m.text));
        }
        if text == "Principal" {
            return Ok(TypeName::Decl(self.principal));
        }
        if let Some(&t) = self.module(module).types.get(text) {
            return Ok(TypeName::Decl(t));
        }
        if let Some(&t) = self.module(PRELUDE).types.get(text) {
            return Ok(TypeName::Decl(t));
        }
        if let Some(b) = Base::from_name(text) {
            return Ok(TypeName::Base(b));
        }
        Ok(match text {
            "Decimal" => TypeName::Decimal,
            "Set" => TypeName::Set,
            "Group" => TypeName::Group,
            "Tag" => TypeName::Tag,
            _ => return Err(no_type(text)),
        })
    }

    /// Resolves a lowercase schema name seen from `module`.
    pub fn lookup_value(&self, module: ModuleId, name: &ast::QName) -> Result<ValueDef, String> {
        let text = name.name.text.as_str();
        let mid = match &name.module {
            Some(m) => self
                .module_named(&m.text)
                .ok_or_else(|| format!("no module `{}`", m.text))?,
            None => module,
        };
        self.module(mid)
            .values
            .get(text)
            .copied()
            .ok_or_else(|| match &name.module {
                Some(m) => format!("module `{}` declares no `{text}`", m.text),
                None => format!("no `{text}` in scope"),
            })
    }

    /// Returns the relation variable `name` names among those a later operator adds.
    fn relation_ahead(&self, module: ModuleId, name: &ast::QName) -> Option<RelId> {
        let m = match &name.module {
            Some(m) => self.module_named(&m.text)?,
            None => module,
        };
        self.ahead
            .iter()
            .position(|(am, n)| *am == m && *n == name.name.text)
            .map(|i| RelId(u32::MAX - i as u32))
    }

    pub fn lookup_relation(&self, module: ModuleId, name: &ast::QName) -> Result<RelId, String> {
        match self.lookup_value(module, name)? {
            ValueDef::Relation(r) => Ok(r),
            _ => Err(format!("`{}` is not a relation", name.name.text)),
        }
    }

    /// Resolves the constructor selection after a relation: constructors of its element type or one named set.
    pub fn relation_sel(&self, rel: RelId, sel: &ast::CtorSel) -> Result<CtorSet, String> {
        let r = self.relation(rel);
        let def = self.ty(r.elem_def);
        let one = |name: &str| -> Result<CtorSet, String> {
            if let Some(i) = def.ctor_index(name) {
                return Ok(vec![i]);
            }
            if let Some(s) = r.sets.iter().find(|s| s.name == name) {
                return Ok(s.ctors.clone());
            }
            Err(format!(
                "`{name}` is neither a constructor of `{}` nor a named set of `{}`",
                def.name, r.name
            ))
        };
        match sel {
            ast::CtorSel::One(c) => one(&c.text),
            ast::CtorSel::Many(cs) => {
                let mut out = BTreeSet::new();
                for c in cs {
                    match def.ctor_index(&c.text) {
                        Some(i) => {
                            if !out.insert(i) {
                                return Err(format!("`{}` is listed twice", c.text));
                            }
                        }
                        None => {
                            return Err(format!(
                                "`{}` is not a constructor of `{}`",
                                c.text, def.name
                            ));
                        }
                    }
                }
                Ok(out.into_iter().collect())
            }
        }
    }

    /// Resolves a constructor selection at the type level, where a named set is any relation's set of that name.
    pub fn type_sel(&self, def: TypeId, sel: &ast::CtorSel) -> Result<CtorSet, String> {
        let t = self.ty(def);
        match sel {
            ast::CtorSel::One(c) => {
                if let Some(i) = t.ctor_index(&c.text) {
                    return Ok(vec![i]);
                }
                self.relations
                    .iter()
                    .filter(|r| r.elem_def == def)
                    .flat_map(|r| r.sets.iter())
                    .find(|s| s.name == c.text)
                    .map(|s| s.ctors.clone())
                    .ok_or_else(|| {
                        format!(
                            "`{}` is neither a constructor nor a named set of `{}`",
                            c.text, t.name
                        )
                    })
            }
            ast::CtorSel::Many(cs) => {
                let mut out = BTreeSet::new();
                for c in cs {
                    let i = t.ctor_index(&c.text).ok_or_else(|| {
                        format!("`{}` is not a constructor of `{}`", c.text, t.name)
                    })?;
                    if !out.insert(i) {
                        return Err(format!("`{}` is listed twice", c.text));
                    }
                }
                Ok(out.into_iter().collect())
            }
        }
    }

    /// Returns `None` for a selection naming every constructor, so `T.(every one)` is `T`.
    pub fn normalize_ctors(&self, def: TypeId, set: CtorSet) -> Option<CtorSet> {
        (set.len() != self.ty(def).ctors().len()).then_some(set)
    }

    /// Returns whether a value of `ty` can hold a value of declared type `def`.
    pub fn reaches(&self, ty: &Ty, def: TypeId) -> bool {
        let mut defs = BTreeSet::new();
        self.reached(ty, &mut defs, &mut BTreeSet::new());
        defs.contains(&def)
    }

    /// Add to `defs` each declared type a value of `ty` can hold, through its type arguments, its
    /// structure or the fields of the types it names, and to `rels` each relation a key it holds
    /// names.
    pub fn reached(&self, ty: &Ty, defs: &mut BTreeSet<TypeId>, rels: &mut BTreeSet<RelId>) {
        match ty {
            Ty::Data { def, args, .. } => {
                for a in args {
                    self.reached(a, defs, rels);
                }
                if !defs.insert(*def) {
                    return;
                }
                match &self.ty(*def).kind {
                    TypeDefKind::Sum { header, ctors, .. } => header
                        .iter()
                        .chain(ctors.iter().flat_map(|c| &c.fields))
                        .for_each(|f| self.reached(&f.ty, defs, rels)),
                    TypeDefKind::Newtype { base, .. } => self.reached(base, defs, rels),
                }
            }
            Ty::Key { rel, .. } => {
                rels.insert(*rel);
            }
            Ty::Record(fs) => fs.iter().for_each(|(_, t)| self.reached(t, defs, rels)),
            Ty::Sum { ctors, .. } => ctors
                .iter()
                .flat_map(|(_, fs)| fs)
                .for_each(|(_, t)| self.reached(t, defs, rels)),
            Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => self.reached(t, defs, rels),
            Ty::Group(a, b) | Ty::Fn(a, b) => {
                self.reached(a, defs, rels);
                self.reached(b, defs, rels);
            }
            Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => {}
        }
    }

    /// Returns whether `ty` is ordered, taking every type parameter as ordered.
    pub fn is_ordered(&self, ty: &Ty) -> bool {
        self.is_ordered_with(ty, &|_| true)
    }

    /// Returns whether `ty` is ordered, where `param` says whether type parameter `i` is.
    pub fn is_ordered_with(&self, ty: &Ty, param: &dyn Fn(u32) -> bool) -> bool {
        Order {
            schema: self,
            definitions: BTreeMap::new(),
            keys: BTreeMap::new(),
            active_definitions: BTreeSet::new(),
            active_keys: BTreeSet::new(),
        }
        .parameters(ty)
        .is_some_and(|required| required.into_iter().all(param))
    }

    pub fn show_ty(&self, ty: &Ty) -> String {
        self.show_ty_with(ty, &[])
    }

    pub fn show_ty_with(&self, ty: &Ty, params: &[String]) -> String {
        let atom = |t: &Ty| {
            let s = self.show_ty_with(t, params);
            let simple = match t {
                Ty::Data { args, .. } => args.is_empty(),
                Ty::Base(Base::Decimal(_))
                | Ty::Key { .. }
                | Ty::Set(_)
                | Ty::NonEmpty(_)
                | Ty::Tag(_)
                | Ty::Group(..)
                | Ty::Fn(..)
                | Ty::Sum { .. } => false,
                _ => true,
            };
            if simple { s } else { format!("({s})") }
        };
        let sel = |def: TypeId, ctors: &Option<CtorSet>| match ctors {
            None => String::new(),
            Some(cs) => {
                let names: Vec<&str> = cs
                    .iter()
                    .map(|&i| self.ty(def).ctors()[i as usize].name.as_str())
                    .collect();
                if names.len() == 1 {
                    format!(".{}", names[0])
                } else {
                    format!(".({})", names.join(" | "))
                }
            }
        };
        let qualifier = |m: ModuleId| {
            if m != PRELUDE && self.module(m).engine {
                format!("{}::", self.module(m).name)
            } else {
                String::new()
            }
        };
        match ty {
            Ty::Base(Base::Decimal(s)) => format!("Decimal {s}"),
            Ty::Base(b) => format!("{b:?}"),
            Ty::Data { def, args, ctors } => {
                let t = self.ty(*def);
                let mut s = format!("{}{}{}", qualifier(t.module), t.name, sel(*def, ctors));
                for a in args {
                    s.push(' ');
                    s.push_str(&atom(a));
                }
                s
            }
            Ty::Param(i) => params
                .get(*i as usize)
                .cloned()
                .unwrap_or_else(|| format!("t{i}")),
            Ty::Key { rel, ctors } => match self.ahead_relation(*rel) {
                Some((m, name)) => format!("Key {}{name}", qualifier(*m)),
                None => {
                    let r = self.relation(*rel);
                    format!(
                        "Key {}{}{}",
                        qualifier(r.module),
                        r.name,
                        sel(r.elem_def, ctors)
                    )
                }
            },
            Ty::Set(t) | Ty::NonEmpty(t) => format!("Set {}", atom(t)),
            Ty::Group(k, t) => format!("Group {} {}", atom(k), atom(t)),
            Ty::Tag(t) => format!("Tag {}", atom(t)),
            Ty::Record(fs) => {
                let fs: Vec<String> = fs
                    .iter()
                    .map(|(n, t)| format!("{n}: {}", self.show_ty_with(t, params)))
                    .collect();
                format!("{{ {} }}", fs.join(", "))
            }
            Ty::Sum { ctors, sel } => {
                let cs: Vec<String> = ctors
                    .iter()
                    .map(|(c, fs)| {
                        if fs.is_empty() {
                            c.clone()
                        } else {
                            let fs: Vec<String> = fs
                                .iter()
                                .map(|(n, t)| format!("{n}: {}", self.show_ty_with(t, params)))
                                .collect();
                            format!("{c} {{ {} }}", fs.join(", "))
                        }
                    })
                    .collect();
                let whole = cs.join(" | ");
                match sel {
                    None => whole,
                    Some(set) => {
                        let names: Vec<&str> =
                            set.iter().map(|&i| ctors[i as usize].0.as_str()).collect();
                        if names.len() == 1 {
                            format!("({whole}).{}", names[0])
                        } else {
                            format!("({whole}).({})", names.join(" | "))
                        }
                    }
                }
            }
            Ty::Fn(a, b) => format!("{} -> {}", atom(a), self.show_ty_with(b, params)),
            Ty::Unit => "()".to_string(),
            Ty::Change => "a write".to_string(),
        }
    }
}

type OrderParams = BTreeSet<u32>;

struct Order<'s> {
    schema: &'s Schema,
    definitions: BTreeMap<TypeId, Option<OrderParams>>,
    keys: BTreeMap<RelId, Option<OrderParams>>,
    active_definitions: BTreeSet<TypeId>,
    active_keys: BTreeSet<RelId>,
}

impl Order<'_> {
    fn parameters(&mut self, ty: &Ty) -> Option<OrderParams> {
        match ty {
            Ty::Base(base) => base.is_ordered().then(OrderParams::new),
            Ty::Param(index) => Some([*index].into()),
            Ty::Data { def, args, .. } => self.definition(*def)?.into_iter().try_fold(
                OrderParams::new(),
                |mut required, index| {
                    required.extend(self.parameters(args.get(index as usize)?)?);
                    Some(required)
                },
            ),
            Ty::Key { rel, .. } => self.key(*rel),
            Ty::Tag(ty) => match ty.as_ref() {
                Ty::Data { def, .. } => self.schema.ty(*def).ordered.then(OrderParams::new),
                _ => None,
            },
            Ty::Set(_)
            | Ty::NonEmpty(_)
            | Ty::Group(..)
            | Ty::Record(_)
            | Ty::Sum { .. }
            | Ty::Fn(..)
            | Ty::Unit
            | Ty::Change => None,
        }
    }

    fn definition(&mut self, def: TypeId) -> Option<OrderParams> {
        if let Some(required) = self.definitions.get(&def) {
            return required.clone();
        }
        if !self.active_definitions.insert(def) {
            return None;
        }
        let declaration = self.schema.ty(def);
        let required = match &declaration.kind {
            TypeDefKind::Newtype { base, .. } => self.parameters(base),
            TypeDefKind::Sum { header, ctors, .. } if declaration.ordered => header
                .iter()
                .chain(ctors.iter().flat_map(|ctor| &ctor.fields))
                .try_fold(OrderParams::new(), |mut required, field| {
                    required.extend(self.parameters(&field.ty)?);
                    Some(required)
                }),
            TypeDefKind::Sum { .. } => None,
        };
        assert!(
            self.active_definitions.remove(&def),
            "an ordering declaration remains active until its requirements are complete"
        );
        self.definitions.insert(def, required.clone());
        required
    }

    fn key(&mut self, rel: RelId) -> Option<OrderParams> {
        if let Some(required) = self.keys.get(&rel) {
            return required.clone();
        }
        if !self.active_keys.insert(rel) {
            return None;
        }
        let relation = self.schema.relation(rel);
        let definition = self.schema.ty(relation.elem_def);
        let required = relation
            .key
            .iter()
            .try_fold(OrderParams::new(), |mut required, name| {
                let field = definition.field(0, name)?;
                if !matches!(&field.ty, Ty::Key { rel: own, .. } if *own == rel) {
                    required.extend(self.parameters(&field.ty.subst(relation.elem_args()))?);
                }
                Some(required)
            });
        assert!(
            self.active_keys.remove(&rel),
            "an ordering key remains active until its requirements are complete"
        );
        self.keys.insert(rel, required.clone());
        required
    }
}

impl RelationDef {
    pub fn elem_args(&self) -> &[Ty] {
        match &self.elem {
            Ty::Data { args, .. } => args,
            _ => &[],
        }
    }
}

/// The index a reference field of a relation variable implies.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ImpliedIndex {
    /// The constructor whose own field it is; `None` for a header field.
    pub ctor: Option<u32>,
    pub field: String,
    /// The constructors that hold the field.
    pub ctors: CtorSet,
}

/// A field of a relation variable's sum holding a reference: a key of another relation
/// variable, or of its own outside its key, or an `Option` of one.
#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceField {
    /// The constructor whose own field it is; `None` for a header field.
    pub ctor: Option<u32>,
    pub field: String,
    /// Its position in a row of each constructor that holds it.
    pub at: usize,
    /// The constructors that hold the field.
    pub ctors: CtorSet,
    pub target: RelId,
    /// The constructors of `target` it may name; `None` for every one.
    pub target_ctors: Option<CtorSet>,
    /// The field is an `Option` of the key.
    pub optional: bool,
    pub on_delete: OnDelete,
    pub index: ReferenceIndex,
}

/// The index of a relation variable that leads with a reference field over every constructor
/// holding it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceIndex {
    Key,
    /// The declared index at this position among the relation's indexes.
    Declared(usize),
    /// The field's implied index.
    Implied,
}

impl Schema {
    /// Returns each reference field of `rel`, header fields first, then each constructor's own
    /// in declaration order.
    pub fn reference_fields(&self, rel: RelId) -> Vec<ReferenceField> {
        let r = self.relation(rel);
        let def = self.ty(r.elem_def);
        let all: CtorSet = (0..def.ctors().len() as u32).collect();
        let header = def.header().iter().enumerate().map(|(at, f)| (None, at, f));
        let own = def.ctors().iter().enumerate().flat_map(|(c, d)| {
            d.fields
                .iter()
                .enumerate()
                .map(move |(at, f)| (Some(c as u32), def.header().len() + at, f))
        });
        header
            .chain(own)
            .filter_map(|(ctor, at, f)| {
                let (target, target_ctors, optional) = self.reference(rel, f)?;
                let ctors = ctor.map_or_else(|| all.clone(), |c| vec![c]);
                let index = self.leading(rel, &f.name, &ctors);
                Some(ReferenceField {
                    ctor,
                    field: f.name.clone(),
                    at,
                    ctors,
                    target,
                    target_ctors,
                    optional,
                    on_delete: f.on_delete.clone(),
                    index,
                })
            })
            .collect()
    }

    /// Returns the index each reference field of `rel` implies, header fields first, then each
    /// constructor's own in declaration order; a field whose constructors a declared key,
    /// unique or index leading with it covers implies none.
    pub fn implied_indexes(&self, rel: RelId) -> Vec<ImpliedIndex> {
        self.reference_fields(rel)
            .into_iter()
            .filter(|f| f.index == ReferenceIndex::Implied)
            .map(|f| ImpliedIndex {
                ctor: f.ctor,
                field: f.field,
                ctors: f.ctors,
            })
            .collect()
    }

    /// Returns whether key field `f` of `rel` is a `Key` of `rel` itself.
    pub fn self_key(&self, rel: RelId, f: &str) -> bool {
        let r = self.relation(rel);
        matches!(
            self.ty(r.elem_def).field(0, f).map(|d| &d.ty),
            Some(Ty::Key { rel: own, .. }) if *own == rel
        )
    }

    /// Returns the target, its constructor selection and whether the field is optional, if
    /// field `f` of `rel`'s sum holds a reference.
    fn reference(&self, rel: RelId, f: &FieldDef) -> Option<(RelId, Option<CtorSet>, bool)> {
        let r = self.relation(rel);
        let option = self.prelude_type("Option");
        let (key, optional) = match f.ty.subst(r.elem_args()) {
            k @ Ty::Key { .. } => (k, false),
            Ty::Data { def, mut args, .. } if def == option && args.len() == 1 => {
                (args.pop()?, true)
            }
            _ => return None,
        };
        match key {
            Ty::Key { rel: target, ctors } if !(target == rel && r.key.contains(&f.name)) => {
                Some((target, ctors, optional))
            }
            _ => None,
        }
    }

    /// Returns the key, else the first declared key, unique or index of `rel` that has `field`
    /// as its leading field and covers every constructor of `ctors`, else the implied index.
    fn leading(&self, rel: RelId, field: &str, ctors: &CtorSet) -> ReferenceIndex {
        let r = self.relation(rel);
        if r.key.first().is_some_and(|k| k == field) {
            return ReferenceIndex::Key;
        }
        r.indexes
            .iter()
            .position(|ix| {
                let leads = matches!(
                    ix.fields.first().map(|e| &e.kind),
                    Some(ast::ExprKind::Name(ast::QName { module: None, name })) if name.text == field
                );
                let covers = ix
                    .scope
                    .as_ref()
                    .is_none_or(|s| ctors.iter().all(|c| s.contains(c)));
                leads && covers
            })
            .map_or(ReferenceIndex::Implied, ReferenceIndex::Declared)
    }
}

/// Where a type expression is written, which decides the forms it may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeMode {
    /// The type of a field of a declared type: named types, type variables and `Key` types only.
    Field,
    /// A signature, where anonymous records and sums, sets and functions are allowed.
    Signature,
}

#[derive(Debug)]
pub struct TypeScope<'a> {
    pub module: ModuleId,
    pub params: &'a [String],
    pub mode: TypeMode,
}

struct Builder<'a> {
    schema: Schema,
    diags: Vec<FileDiag>,
    files: &'a [(FileId, &'a ast::SchemaFile)],
    /// Spans of `Span a` uses whose argument must be ordered, checked once every type is known.
    ordered_args: Vec<(Ty, FileId, Span)>,
    ahead: &'a [Ahead],
}

/// Builds the schema the given module files declare, over the prelude.
pub fn build(files: &[(FileId, ast::SchemaFile)]) -> Result<Schema, Vec<FileDiag>> {
    build_ahead(files, &[])
}

/// A relation variable a later operator of the migration being applied adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ahead {
    pub module: String,
    pub relation: String,
    /// Whether the migration also adds the relation variable's type.
    pub typed: bool,
}

/// Builds the schema as `build` does, where a `Key r` type may also name each relation variable
/// `ahead` lists that the files do not declare, and select the constructors of one whose type the
/// migration adds before they resolve.
pub fn build_ahead(
    files: &[(FileId, ast::SchemaFile)],
    ahead: &[Ahead],
) -> Result<Schema, Vec<FileDiag>> {
    let all: Vec<_> = BUILT_IN
        .iter()
        .chain(files)
        .map(|(id, f)| (*id, f))
        .collect();
    let mut b = Builder {
        schema: Schema {
            modules: Vec::new(),
            types: Vec::new(),
            relations: Vec::new(),
            functions: Vec::new(),
            constraints: Vec::new(),
            checks: Vec::new(),
            aggregates: Vec::new(),
            principal: TypeId(0),
            ids: crate::ids::CatalogIds::default(),
            stages: BTreeMap::new(),
            value_stages: BTreeMap::new(),
            ahead: Vec::new(),
            late_sels: Vec::new(),
        },
        diags: Vec::new(),
        files: &all,
        ahead,
        ordered_args: Vec::new(),
    };
    b.run();
    if !b.diags.is_empty() {
        return Err(b.diags);
    }
    let names = crate::resolve::check_schema(&b.schema);
    if names.is_empty() {
        b.schema.types.push(tag_former());
        b.schema.ids = crate::ids::declared(&b.schema, files);
        Ok(b.schema)
    } else {
        Err(names)
    }
}

/// Returns the error for type name `text`, naming the prelude type that another language's
/// spelling `text` usually means.
fn no_type(text: &str) -> String {
    let meant = match text {
        "Int" | "Integer" | "Long" | "BigInt" | "Int64" => "I64",
        "Int32" => "I32",
        "Float" | "Double" => "F64",
        "String" | "Str" | "Varchar" => "Text",
        "Boolean" => "Bool",
        "Blob" => "Bytes",
        "Timestamp" => "Instant",
        _ => return format!("no type `{text}`"),
    };
    format!("no type `{text}`; did you mean `{meant}`?")
}

/// Returns the prelude's declaration of the type former `Tag`, with no constructors of its own.
/// No module scope holds it: the name `Tag` resolves to the former, never to this declaration.
fn tag_former() -> TypeDef {
    TypeDef {
        name: "Tag".to_string(),
        module: PRELUDE,
        params: vec!["a".to_string()],
        ordered: false,
        kind: TypeDefKind::Sum {
            header: Vec::new(),
            ctors: Vec::new(),
            record: false,
        },
        file: FileId::PRELUDE,
        span: Span::default(),
    }
}

/// The prelude and the engine's own module files, parsed once.
static BUILT_IN: LazyLock<Vec<(FileId, ast::SchemaFile)>> = LazyLock::new(|| {
    let prelude = parse::parse_schema(PRELUDE_SRC, Options::default())
        .ok()
        .expect("the prelude parses");
    let mut all = vec![(FileId::PRELUDE, prelude)];
    all.extend(engine_files().into_iter().map(|f| (FileId::ENGINE, f)));
    all
});

/// The module files of the engine's own declarations.
fn engine_files() -> Vec<ast::SchemaFile> {
    let items = parse::parse_example(ENGINE_SRC, Options::default())
        .ok()
        .expect("the engine's declarations parse");
    let mut files: Vec<ast::SchemaFile> = Vec::new();
    for item in items {
        match item {
            ast::ExampleItem::Module(m) => files.push(ast::SchemaFile {
                module: m,
                decls: Vec::new(),
            }),
            ast::ExampleItem::Decl(d) => files
                .last_mut()
                .expect("the engine's declarations start with a module")
                .decls
                .push(d),
            _ => panic!("the engine's declarations hold only modules and declarations"),
        }
    }
    files
}

pub fn has_param(ty: &Ty) -> bool {
    match ty {
        Ty::Param(_) => true,
        Ty::Data { args, .. } => args.iter().any(has_param),
        Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => has_param(t),
        Ty::Group(k, t) | Ty::Fn(k, t) => has_param(k) || has_param(t),
        Ty::Record(fs) => fs.iter().any(|(_, t)| has_param(t)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .any(|(_, fs)| fs.iter().any(|(_, t)| has_param(t))),
        Ty::Base(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => false,
    }
}

fn mentions_key(ty: &Ty) -> bool {
    match ty {
        Ty::Key { .. } => true,
        Ty::Data { args, .. } => args.iter().any(mentions_key),
        Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => mentions_key(t),
        Ty::Group(k, t) | Ty::Fn(k, t) => mentions_key(k) || mentions_key(t),
        Ty::Record(fs) => fs.iter().any(|(_, t)| mentions_key(t)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .any(|(_, fs)| fs.iter().any(|(_, t)| mentions_key(t))),
        Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => false,
    }
}

fn collect_defs(ty: &Ty, out: &mut Vec<TypeId>) {
    if let Ty::Data { def, args, .. } = ty {
        out.push(*def);
        for a in args {
            collect_defs(a, out);
        }
    }
}
