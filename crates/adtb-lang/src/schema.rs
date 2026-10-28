use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

use crate::ast;
use crate::parse::{self, Options};
use crate::span::{Diag, FileDiag, FileId, Span};
use crate::types::*;

const PRELUDE_SRC: &str = include_str!("prelude.adt");

/// The engine's declarations of its `catalog` and `sys` modules.
pub const ENGINE_SRC: &str = include_str!("catalog.adt");

/// The modules the engine declares, which no user file may name.
pub const ENGINE_MODULES: [&str; 2] = ["catalog", "sys"];

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;

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
        domain: Option<ast::Expr>,
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
    Expr(ast::Expr),
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

    /// Resolves a type name seen from `module`: the module, then the root `Principal`, then the prelude and built-ins.
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
        if let Some(&t) = self.module(module).types.get(text) {
            return Ok(TypeName::Decl(t));
        }
        if text == "Principal" {
            return Ok(TypeName::Decl(self.principal));
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

impl Builder<'_> {
    fn err(&mut self, file: FileId, span: Span, msg: impl Into<String>) {
        self.diags.push(FileDiag {
            file,
            diag: Diag::new(span, msg),
        });
    }

    fn module_id(&mut self, name: &str) -> ModuleId {
        if let Some(m) = self.schema.module_named(name) {
            return m;
        }
        self.schema.modules.push(ModuleDef {
            name: name.to_string(),
            engine: self.schema.modules.is_empty() || ENGINE_MODULES.contains(&name),
            types: BTreeMap::new(),
            values: BTreeMap::new(),
        });
        ModuleId(self.schema.modules.len() as u32 - 1)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "collecting names and each later pass would have to be a function"
    )]
    fn run(&mut self) {
        let files = self.files;
        let mut principal = None;
        let mut principal_decl = None;
        // Collect every name, so later passes may refer forward.
        let mut type_decls = Vec::new();
        let mut rel_decls = Vec::new();
        let mut set_decls = Vec::new();
        let mut fn_decls = Vec::new();
        let mut stores = Vec::new();
        for (file, sf) in files {
            let file = *file;
            let prelude = file == FileId::PRELUDE;
            let engine = file.is_engine();
            let module = self.module_id(&sf.module.text);
            if !engine && self.schema.module(module).engine {
                let msg = format!("no module may be named `{}`", sf.module.text);
                self.err(file, sf.module.span, msg);
            }
            for decl in &sf.decls {
                match decl {
                    ast::Decl::Type(t) => {
                        let id = TypeId(self.schema.types.len() as u32);
                        if !prelude {
                            self.check_type_name(file, &t.name, principal.is_some());
                        }
                        if !prelude && t.name.text == "Principal" {
                            principal = Some(id);
                            principal_decl = Some((file, t.name.span));
                        } else if self.schema.modules[module.0 as usize]
                            .types
                            .insert(t.name.text.clone(), id)
                            .is_some()
                        {
                            self.err(
                                file,
                                t.name.span,
                                format!("type `{}` is declared twice", t.name.text),
                            );
                        }
                        let skeleton = self.type_skeleton(module, file, t);
                        self.schema.types.push(skeleton);
                        type_decls.push((file, module, id, t));
                    }
                    ast::Decl::Relation(r) => {
                        let id = RelId(self.schema.relations.len() as u32);
                        let home = match &r.name.module {
                            Some(m) if engine => self.module_id(&m.text),
                            Some(_) => {
                                self.err(
                                    file,
                                    r.name.span(),
                                    "a relation name is qualified only in the engine's own declarations",
                                );
                                module
                            }
                            None => module,
                        };
                        self.declare_value(file, home, &r.name.name, ValueDef::Relation(id));
                        self.schema.relations.push(RelationDef {
                            name: r.name.name.text.clone(),
                            module: home,
                            elem: Ty::Unit,
                            elem_def: TypeId(u32::MAX),
                            key: Vec::new(),
                            generate: None,
                            indexes: Vec::new(),
                            defaults: Vec::new(),
                            computed: Vec::new(),
                            sets: Vec::new(),
                            volatile: false,
                            file,
                            span: r.span,
                        });
                        rel_decls.push((file, module, id, r));
                    }
                    ast::Decl::Set(s) => set_decls.push((file, module, s)),
                    ast::Decl::Fn(f) => {
                        let id = FnId(self.schema.functions.len() as u32);
                        self.check_fn_name(file, &f.name);
                        self.declare_value(file, module, &f.name, ValueDef::Function(id));
                        self.schema.functions.push(FunctionDef {
                            name: f.name.text.clone(),
                            module,
                            type_params: Vec::new(),
                            params: Vec::new(),
                            result: Ty::Unit,
                            body: f.body.clone(),
                            stored: None,
                            file,
                            span: f.span,
                        });
                        fn_decls.push((file, module, id, f));
                    }
                    ast::Decl::Constraint(c) => {
                        let id = self.schema.constraints.len() as u32;
                        self.declare_value(file, module, &c.name, ValueDef::Constraint(id));
                        self.schema.constraints.push(ConstraintDef {
                            name: c.name.text.clone(),
                            module,
                            query: c.query.clone(),
                            file,
                        });
                    }
                    ast::Decl::Check(c) => {
                        let id = self.schema.checks.len() as u32;
                        self.declare_value(file, module, &c.name, ValueDef::Check(id));
                        self.schema.checks.push(CheckDef {
                            name: c.name.text.clone(),
                            module,
                            relation: RelId(u32::MAX),
                            ctors: None,
                            predicate: c.predicate.clone(),
                            file,
                        });
                    }
                    ast::Decl::Aggregate(a) => {
                        let id = self.schema.aggregates.len() as u32;
                        self.check_fn_name(file, &a.name);
                        self.declare_value(file, module, &a.name, ValueDef::Aggregate(id));
                        self.schema.aggregates.push(AggregateDef {
                            name: a.name.text.clone(),
                            module,
                            decl: a.clone(),
                            params: Vec::new(),
                            result: Ty::Unit,
                            state: Ty::Unit,
                            file,
                        });
                    }
                    ast::Decl::Store(s) => stores.push((file, module, s)),
                }
            }
        }
        self.declare_ahead();
        self.schema.principal = principal.unwrap_or_else(|| self.schema.prelude_type("Principal"));

        for &(file, module, id, r) in &rel_decls {
            self.relation_head(file, module, id, r);
        }
        for &(file, module, s) in &set_decls {
            self.named_set(file, module, s);
        }
        self.check_set_consistency();
        for &(file, module, id, t) in &type_decls {
            self.type_body(file, module, id, t);
        }
        if let Some((file, span)) = principal_decl {
            self.check_principal(file, span);
        }
        for &(file, module, id, r) in &rel_decls {
            self.relation_body(file, module, id, r);
        }
        for &(file, module, id, f) in &fn_decls {
            self.fn_signature(file, module, id, f);
        }
        for i in 0..self.schema.aggregates.len() {
            self.aggregate_signature(i);
        }
        for (file, module, s) in stores {
            self.store(file, module, s);
        }
        self.checks();
        self.check_recursion();
        self.check_references();
        self.check_ordered();
    }

    fn declare_ahead(&mut self) {
        for a in self.ahead {
            let Some(module) = self.schema.module_named(&a.module) else {
                continue;
            };
            if a.typed {
                self.schema.late_sels.push((module, a.relation.clone()));
            }
            if self.schema.module(module).values.contains_key(&a.relation) {
                continue;
            }
            self.schema.ahead.push((module, a.relation.clone()));
        }
    }

    fn check_type_name(&mut self, file: FileId, name: &ast::Ident, principal_seen: bool) {
        let text = name.text.as_str();
        if RESERVED_TYPE_NAMES.contains(&text) {
            self.err(file, name.span, format!("`{text}` is a reserved type name"));
        } else if text == "Principal" {
            if principal_seen {
                self.err(file, name.span, "the schema declares `Principal` once");
            }
        } else if is_prelude_type(text) {
            self.err(file, name.span, format!("`{text}` is a prelude type"));
        }
    }

    /// Refuse a declared `Principal` other than a sum of `EndUser`, `Service { name: Text }` and
    /// `Anonymous`, with no header fields.
    fn check_principal(&mut self, file: FileId, span: Span) {
        let def = self.schema.ty(self.schema.principal);
        let shaped = |name: &str, fields: Option<&[(&str, Ty)]>| {
            def.ctors().iter().any(|c| {
                c.name == name
                    && fields.is_none_or(|want| {
                        c.fields.len() == want.len()
                            && c.fields
                                .iter()
                                .zip(want)
                                .all(|(f, (n, t))| f.name == *n && f.ty == *t)
                    })
            })
        };
        let ok = matches!(def.kind, TypeDefKind::Sum { .. })
            && def.header().is_empty()
            && def.ctors().len() == 3
            && shaped("EndUser", None)
            && shaped("Service", Some(&[("name", Ty::Base(Base::Text))]))
            && shaped("Anonymous", Some(&[]));
        if !ok {
            self.err(
                file,
                span,
                "`Principal` has exactly the constructors `EndUser`, `Service { name: Text }` and \
                 `Anonymous`",
            );
        }
    }

    fn check_fn_name(&mut self, file: FileId, name: &ast::Ident) {
        if PRELUDE_FUNCTIONS.contains(&name.text.as_str()) {
            self.err(
                file,
                name.span,
                format!("`{}` is a prelude function", name.text),
            );
        }
    }

    fn declare_value(&mut self, file: FileId, module: ModuleId, name: &ast::Ident, v: ValueDef) {
        let m = &mut self.schema.modules[module.0 as usize];
        if m.values.insert(name.text.clone(), v).is_some() {
            let msg = format!("`{}` is declared twice in module `{}`", name.text, m.name);
            self.err(file, name.span, msg);
        }
    }

    /// The constructor and header names of a type, with field types left for later.
    fn type_skeleton(&mut self, module: ModuleId, file: FileId, t: &ast::TypeDecl) -> TypeDef {
        let mut params = Vec::new();
        for p in &t.params {
            if params.contains(&p.text) {
                self.err(
                    file,
                    p.span,
                    format!("type parameter `{}` is declared twice", p.text),
                );
            }
            params.push(p.text.clone());
        }
        let field_names = |fs: &[ast::FieldType]| -> Vec<FieldDef> {
            fs.iter()
                .map(|f| FieldDef {
                    name: f.name.text.clone(),
                    ty: Ty::Unit,
                    on_delete: OnDelete::Restrict,
                    span: f.name.span,
                })
                .collect()
        };
        let kind = match &t.body {
            ast::TypeBody::New(..) => {
                if t.header.is_some() {
                    self.err(file, t.name.span, "a newtype has no header");
                }
                TypeDefKind::Newtype {
                    base: Ty::Unit,
                    domain: None,
                }
            }
            ast::TypeBody::Record(fs) => {
                if t.header.is_some() {
                    self.err(
                        file,
                        t.name.span,
                        "a header is allowed only when the body lists constructors",
                    );
                }
                TypeDefKind::Sum {
                    header: Vec::new(),
                    ctors: vec![CtorDef {
                        name: t.name.text.clone(),
                        fields: field_names(fs),
                        span: t.name.span,
                    }],
                    record: true,
                }
            }
            ast::TypeBody::Ctors(cs) => {
                let mut ctors: Vec<CtorDef> = Vec::new();
                for c in cs {
                    if ctors.iter().any(|d| d.name == c.name.text) {
                        self.err(
                            file,
                            c.name.span,
                            format!("constructor `{}` is listed twice", c.name.text),
                        );
                    }
                    ctors.push(CtorDef {
                        name: c.name.text.clone(),
                        fields: c.record.as_deref().map(field_names).unwrap_or_default(),
                        span: c.name.span,
                    });
                }
                TypeDefKind::Sum {
                    header: t.header.as_deref().map(field_names).unwrap_or_default(),
                    ctors,
                    record: false,
                }
            }
        };
        if let TypeDefKind::Sum { header, ctors, .. } = &kind {
            let mut seen = BTreeSet::new();
            for f in header {
                if !seen.insert(f.name.clone()) {
                    self.err(
                        file,
                        f.span,
                        format!("field `{}` is declared twice", f.name),
                    );
                }
            }
            for c in ctors {
                let mut own = BTreeSet::new();
                for f in &c.fields {
                    if !own.insert(f.name.clone()) {
                        self.err(
                            file,
                            f.span,
                            format!("field `{}` is declared twice", f.name),
                        );
                    } else if seen.contains(&f.name) {
                        self.err(
                            file,
                            f.span,
                            format!("field `{}` of `{}` is also a header field", f.name, c.name),
                        );
                    }
                }
            }
        }
        TypeDef {
            name: t.name.text.clone(),
            module,
            params,
            ordered: t.ordered,
            kind,
            file,
            span: t.span,
        }
    }

    fn relation_head(&mut self, file: FileId, module: ModuleId, id: RelId, r: &ast::RelationDecl) {
        let head = match &r.ty.kind {
            ast::TypeKind::Named {
                name, sel: None, ..
            } => self.schema.lookup_type(module, name),
            _ => Err("a relation's element type is a declared sum or record".to_string()),
        };
        match head {
            Ok(TypeName::Decl(t)) if matches!(self.schema.ty(t).kind, TypeDefKind::Sum { .. }) => {
                self.schema.relations[id.0 as usize].elem_def = t;
            }
            Ok(_) => self.err(
                file,
                r.ty.span,
                "a relation's element type is a declared sum or record",
            ),
            Err(e) => self.err(file, r.ty.span, e),
        }
    }

    fn named_set(&mut self, file: FileId, module: ModuleId, s: &ast::SetDecl) {
        let qname = ast::QName {
            module: None,
            name: s.relation.clone(),
        };
        let rel = match self.schema.lookup_relation(module, &qname) {
            Ok(r) => r,
            Err(e) => return self.err(file, s.relation.span, e),
        };
        let elem_def = self.schema.relation(rel).elem_def;
        if elem_def.0 == u32::MAX {
            return;
        }
        let def = self.schema.ty(elem_def);
        let mut ctors = BTreeSet::new();
        let mut bad = Vec::new();
        for m in &s.members {
            match def.ctor_index(&m.text) {
                Some(i) => {
                    if !ctors.insert(i) {
                        bad.push((m.span, format!("`{}` is listed twice", m.text)));
                    }
                }
                None => bad.push((
                    m.span,
                    format!("`{}` is not a constructor of `{}`", m.text, def.name),
                )),
            }
        }
        let clash = def.ctor_index(&s.name.text).is_some();
        for (span, msg) in bad {
            self.err(file, span, msg);
        }
        if clash {
            self.err(
                file,
                s.name.span,
                format!("`{}` is already a constructor name", s.name.text),
            );
        }
        let r = &mut self.schema.relations[rel.0 as usize];
        if r.sets.iter().any(|x| x.name == s.name.text) {
            let msg = format!("`{}` declares set `{}` twice", r.name, s.name.text);
            return self.err(file, s.name.span, msg);
        }
        r.sets.push(NamedSet {
            name: s.name.text.clone(),
            ctors: ctors.into_iter().collect(),
        });
    }

    fn check_set_consistency(&mut self) {
        let mut seen: HashMap<(TypeId, String), (CtorSet, String)> = HashMap::new();
        let mut errs = Vec::new();
        for r in &self.schema.relations {
            for s in &r.sets {
                let k = (r.elem_def, s.name.clone());
                match seen.get(&k) {
                    Some((cs, other)) if *cs != s.ctors => errs.push((
                        r.file,
                        r.span,
                        format!(
                            "named set `{}` of `{}` has other constructors than the set of that name of `{other}`",
                            s.name, r.name
                        ),
                    )),
                    Some(_) => {}
                    None => {
                        seen.insert(k, (s.ctors.clone(), r.name.clone()));
                    }
                }
            }
        }
        for (f, s, m) in errs {
            self.err(f, s, m);
        }
    }

    pub fn resolve_type(&mut self, file: FileId, scope: &TypeScope, t: &ast::Type) -> Option<Ty> {
        match resolve_type(&self.schema, scope, t, &mut self.ordered_args, file) {
            Ok(ty) => Some(ty),
            Err((span, msg)) => {
                self.err(file, span, msg);
                None
            }
        }
    }

    fn resolve_fields(
        &mut self,
        file: FileId,
        module: ModuleId,
        params: &[String],
        fields: &[ast::FieldType],
        own: TypeId,
    ) -> Vec<FieldDef> {
        let scope = TypeScope {
            module,
            params,
            mode: TypeMode::Field,
        };
        let mut out = Vec::new();
        for f in fields {
            let ty = self.resolve_type(file, &scope, &f.ty).unwrap_or(Ty::Unit);
            let on_delete = match &f.on_delete {
                None => OnDelete::Restrict,
                Some(a) => {
                    let is_key = matches!(ty, Ty::Key { .. });
                    let is_opt_key = self.is_option_key(&ty);
                    if !is_key && !is_opt_key {
                        self.err(
                            file,
                            f.name.span,
                            "`on delete` is allowed only on a `Key` field or an `Option` of one",
                        );
                    }
                    match &a.action {
                        ast::RefAction::Restrict => OnDelete::Restrict,
                        ast::RefAction::Cascade => OnDelete::Cascade,
                        ast::RefAction::Clear => {
                            if !is_opt_key {
                                self.err(
                                    file,
                                    f.name.span,
                                    "`on delete clear` is allowed only on an `Option (Key ...)` field",
                                );
                            }
                            OnDelete::Clear
                        }
                        ast::RefAction::Retag(c, rec) => {
                            match self.schema.ty(own).ctor_index(&c.text) {
                                Some(i) => OnDelete::Retag(i, rec.clone()),
                                None => {
                                    let msg = format!(
                                        "`{}` is not a constructor of `{}`",
                                        c.text,
                                        self.schema.ty(own).name
                                    );
                                    self.err(file, c.span, msg);
                                    OnDelete::Restrict
                                }
                            }
                        }
                    }
                }
            };
            out.push(FieldDef {
                name: f.name.text.clone(),
                ty,
                on_delete,
                span: f.name.span,
            });
        }
        out
    }

    fn is_option_key(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Data { def, args, .. } => {
                *def == self.schema.prelude_type("Option")
                    && matches!(args.as_slice(), [Ty::Key { .. }])
            }
            _ => false,
        }
    }

    fn type_body(&mut self, file: FileId, module: ModuleId, id: TypeId, t: &ast::TypeDecl) {
        let params = self.schema.ty(id).params.clone();
        let kind = match &t.body {
            ast::TypeBody::New(base, domain) => {
                let scope = TypeScope {
                    module,
                    params: &params,
                    mode: TypeMode::Field,
                };
                let ty = self.resolve_type(file, &scope, base).unwrap_or(Ty::Unit);
                if !matches!(ty, Ty::Base(_)) && ty != Ty::Unit {
                    self.err(file, base.span, "a newtype is over a base type");
                }
                TypeDefKind::Newtype {
                    base: ty,
                    domain: domain.clone(),
                }
            }
            ast::TypeBody::Record(fs) => {
                let fields = self.resolve_fields(file, module, &params, fs, id);
                TypeDefKind::Sum {
                    header: Vec::new(),
                    ctors: vec![CtorDef {
                        name: t.name.text.clone(),
                        fields,
                        span: t.name.span,
                    }],
                    record: true,
                }
            }
            ast::TypeBody::Ctors(cs) => {
                let header = t
                    .header
                    .as_deref()
                    .map(|h| self.resolve_fields(file, module, &params, h, id))
                    .unwrap_or_default();
                let ctors = cs
                    .iter()
                    .map(|c| CtorDef {
                        name: c.name.text.clone(),
                        fields: c
                            .record
                            .as_deref()
                            .map(|fs| self.resolve_fields(file, module, &params, fs, id))
                            .unwrap_or_default(),
                        span: c.name.span,
                    })
                    .collect();
                TypeDefKind::Sum {
                    header,
                    ctors,
                    record: false,
                }
            }
        };
        self.schema.types[id.0 as usize].kind = kind;
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the key, the sets and each clause kind would have to be resolved by their own functions"
    )]
    fn relation_body(&mut self, file: FileId, module: ModuleId, id: RelId, r: &ast::RelationDecl) {
        let elem_def = self.schema.relation(id).elem_def;
        if elem_def.0 == u32::MAX {
            return;
        }
        let scope = TypeScope {
            module,
            params: &[],
            mode: TypeMode::Field,
        };
        if let Some(elem) = self.resolve_type(file, &scope, &r.ty) {
            self.schema.relations[id.0 as usize].elem = elem;
        }
        let def = self.schema.ty(elem_def).clone();
        let mut key: Option<Vec<String>> = None;
        let mut generate = None;
        let mut indexes = Vec::new();
        let mut defaults = Vec::new();
        let mut computed = Vec::new();
        let mut volatile = false;
        let key_candidates: Vec<&FieldDef> = if def.ctors().len() == 1 {
            def.header()
                .iter()
                .chain(def.ctors()[0].fields.iter())
                .collect()
        } else {
            def.header().iter().collect()
        };
        let scope_of = |b: &mut Self, s: &Option<ast::CtorScope>| -> Option<CtorSet> {
            let s = s.as_ref()?;
            let sel = match s {
                ast::CtorScope::One(c) => ast::CtorSel::One(c.clone()),
                ast::CtorScope::Many(cs) => ast::CtorSel::Many(cs.clone()),
            };
            match b.schema.relation_sel(id, &sel) {
                Ok(cs) => Some(cs),
                Err(e) => {
                    b.err(file, r.span, e);
                    None
                }
            }
        };
        for clause in &r.clauses {
            match clause {
                ast::RelationClause::Key(fields, gen_name) => {
                    if key.is_some() {
                        self.err(file, r.name.span(), "a relation declares one key");
                    }
                    let mut names = Vec::new();
                    for f in fields {
                        if !key_candidates.iter().any(|c| c.name == f.text) {
                            self.err(
                                file,
                                f.span,
                                format!(
                                    "key field `{}` is not a header field or a field of the only constructor of `{}`",
                                    f.text, def.name
                                ),
                            );
                        } else if names.contains(&f.text) {
                            self.err(
                                file,
                                f.span,
                                format!("key field `{}` is named twice", f.text),
                            );
                        }
                        names.push(f.text.clone());
                    }
                    if let Some(g) = gen_name {
                        let gen_kind = match g.text.as_str() {
                            "sequence" => Some(Generator::Sequence),
                            "uuidv7" => Some(Generator::Uuidv7),
                            _ => None,
                        };
                        match gen_kind {
                            None => self.err(
                                file,
                                g.span,
                                format!(
                                    "`{}` is not a key generator; use `sequence` or `uuidv7`",
                                    g.text
                                ),
                            ),
                            Some(k) => {
                                let self_key = names.len() == 1
                                    && key_candidates.iter().any(|c| {
                                        c.name == names[0]
                                            && matches!(c.ty, Ty::Key { rel, .. } if rel == id)
                                    });
                                if !self_key {
                                    self.err(
                                        file,
                                        g.span,
                                        "a generated key is one field of type `Key` of its own relation",
                                    );
                                }
                                generate = Some(k);
                            }
                        }
                    }
                    key = Some(names);
                }
                ast::RelationClause::Volatile => {
                    volatile = true;
                    if !file.is_engine() {
                        self.err(
                            file,
                            r.name.span(),
                            "`volatile` appears only in the engine's own declarations",
                        );
                    }
                }
                ast::RelationClause::Unique(fields, s) => {
                    let scope = scope_of(self, s);
                    indexes.push(IndexDef {
                        fields: fields.clone(),
                        scope,
                        interval: false,
                        unique: true,
                    });
                }
                ast::RelationClause::Index {
                    interval,
                    fields,
                    scope: s,
                } => {
                    let scope = scope_of(self, s);
                    indexes.push(IndexDef {
                        fields: fields.clone(),
                        scope,
                        interval: *interval,
                        unique: false,
                    });
                }
                ast::RelationClause::Default { ctor, field, value } => {
                    let ctor_idx = match ctor {
                        None => None,
                        Some(c) => match def.ctor_index(&c.text) {
                            Some(i) => Some(i),
                            None => {
                                self.err(
                                    file,
                                    c.span,
                                    format!("`{}` is not a constructor of `{}`", c.text, def.name),
                                );
                                continue;
                            }
                        },
                    };
                    let field_ty = match ctor_idx {
                        Some(i) => def.field(i, &field.text).map(|f| f.ty.clone()),
                        None => (0..def.ctors().len() as u32)
                            .find_map(|i| def.field(i, &field.text))
                            .map(|f| f.ty.clone()),
                    };
                    let Some(field_ty) = field_ty else {
                        self.err(
                            file,
                            field.span,
                            format!("`{}` has no field `{}`", def.name, field.text),
                        );
                        continue;
                    };
                    let value = match value {
                        ast::DefaultValue::Generate(g) => {
                            if g.text != "clock" {
                                self.err(
                                    file,
                                    g.span,
                                    "a default generates only `clock`; keys generate in the `key` clause",
                                );
                            } else if field_ty != Ty::Base(Base::Instant) {
                                self.err(file, g.span, "`generate clock` fills an `Instant` field");
                            }
                            DefaultValue::Generate(Generator::Clock)
                        }
                        ast::DefaultValue::Expr(e) => DefaultValue::Expr(e.clone()),
                    };
                    if defaults
                        .iter()
                        .any(|d: &DefaultDef| d.ctor == ctor_idx && d.field == field.text)
                    {
                        self.err(file, field.span, "this default is declared twice");
                    }
                    defaults.push(DefaultDef {
                        ctor: ctor_idx,
                        field: field.text.clone(),
                        value,
                    });
                }
                ast::RelationClause::Computed(name, ty, e) => {
                    let scope = TypeScope {
                        module,
                        params: &[],
                        mode: TypeMode::Field,
                    };
                    let ty = self.resolve_type(file, &scope, ty).unwrap_or(Ty::Unit);
                    if (0..def.ctors().len() as u32).any(|i| def.field(i, &name.text).is_some()) {
                        self.err(
                            file,
                            name.span,
                            format!("`{}` already has a field `{}`", def.name, name.text),
                        );
                    }
                    computed.push(ComputedDef {
                        name: name.text.clone(),
                        ty,
                        expr: e.clone(),
                    });
                }
            }
        }
        if key.is_none() {
            self.err(file, r.name.span(), "a relation declares exactly one key");
        }
        let rel = &mut self.schema.relations[id.0 as usize];
        rel.key = key.unwrap_or_default();
        rel.generate = generate;
        rel.indexes = indexes;
        rel.defaults = defaults;
        rel.computed = computed;
        rel.volatile = volatile;
        if let Ty::Data { args, .. } = &rel.elem
            && args.iter().any(has_param)
        {
            let span = rel.span;
            self.err(
                file,
                span,
                "a relation's element type has no free type parameters",
            );
        }
    }

    fn aggregate_signature(&mut self, i: usize) {
        let (file, module, a) = {
            let d = &self.schema.aggregates[i];
            (d.file, d.module, d.decl.clone())
        };
        let scope = TypeScope {
            module,
            params: &[],
            mode: TypeMode::Signature,
        };
        let mut params: Vec<(String, Ty)> = Vec::new();
        for p in &a.params {
            if params.iter().any(|(n, _)| *n == p.name.text) {
                self.err(
                    file,
                    p.name.span,
                    format!("parameter `{}` is declared twice", p.name.text),
                );
            }
            let ty = self.resolve_type(file, &scope, &p.ty).unwrap_or(Ty::Unit);
            params.push((p.name.text.clone(), ty));
        }
        let result = self
            .resolve_type(file, &scope, &a.result)
            .unwrap_or(Ty::Unit);
        let mut fields: Vec<(String, Ty)> = Vec::new();
        for f in &a.state {
            if fields.iter().any(|(n, _)| *n == f.name.text) {
                self.err(
                    file,
                    f.name.span,
                    format!("state field `{}` is declared twice", f.name.text),
                );
            }
            let ty = self.resolve_type(file, &scope, &f.ty).unwrap_or(Ty::Unit);
            fields.push((f.name.text.clone(), ty));
        }
        fields.sort_by(|x, y| x.0.cmp(&y.0));
        let def = &mut self.schema.aggregates[i];
        def.params = params;
        def.result = result;
        def.state = Ty::Record(fields);
    }

    fn fn_signature(&mut self, file: FileId, module: ModuleId, id: FnId, f: &ast::FnDecl) {
        let mut tparams = Vec::new();
        let mut names = Vec::new();
        for tp in &f.type_params {
            if names.contains(&tp.name.text) {
                self.err(
                    file,
                    tp.name.span,
                    format!("type parameter `{}` is declared twice", tp.name.text),
                );
            }
            let mut bounds = Vec::new();
            for b in &tp.bounds {
                match Bound::from_name(&b.text) {
                    Some(k) => bounds.push(k),
                    None => self.err(
                        file,
                        b.span,
                        format!(
                            "`{}` is not a bound; use `Eq`, `Ord`, `Hash` or `Num`",
                            b.text
                        ),
                    ),
                }
            }
            names.push(tp.name.text.clone());
            tparams.push((tp.name.text.clone(), bounds));
        }
        let scope = TypeScope {
            module,
            params: &names,
            mode: TypeMode::Signature,
        };
        let mut params = Vec::new();
        for p in &f.params {
            if params.iter().any(|(n, _): &(String, Ty)| *n == p.name.text) {
                self.err(
                    file,
                    p.name.span,
                    format!("parameter `{}` is declared twice", p.name.text),
                );
            }
            let ty = self.resolve_type(file, &scope, &p.ty).unwrap_or(Ty::Unit);
            params.push((p.name.text.clone(), ty));
        }
        let result = self
            .resolve_type(file, &scope, &f.result)
            .unwrap_or(Ty::Unit);
        let def = &mut self.schema.functions[id.0 as usize];
        def.type_params = tparams;
        def.params = params;
        def.result = result;
    }

    fn store(&mut self, file: FileId, module: ModuleId, s: &ast::StoreDecl) {
        match self.schema.lookup_value(module, &s.view) {
            Ok(ValueDef::Function(f)) => {
                let def = &self.schema.functions[f.0 as usize];
                if !def.params.is_empty() || !def.type_params.is_empty() {
                    return self.err(
                        file,
                        s.view.span(),
                        "a stored view is a function with no parameters",
                    );
                }
                if def.stored.is_some() {
                    return self.err(file, s.view.span(), "this view is stored twice");
                }
                self.schema.functions[f.0 as usize].stored = Some(s.clone());
            }
            Ok(_) => self.err(
                file,
                s.view.span(),
                format!("`{}` is not a function", s.view.name.text),
            ),
            Err(e) => self.err(file, s.view.span(), e),
        }
    }

    fn checks(&mut self) {
        let files = self.files;
        let mut i = 0;
        for (file, sf) in files {
            let module = self.schema.module_named(&sf.module.text).unwrap_or(PRELUDE);
            for decl in &sf.decls {
                let ast::Decl::Check(c) = decl else { continue };
                let resolved = self
                    .schema
                    .lookup_relation(module, &c.on.relation)
                    .and_then(|rel| {
                        let ctors = match &c.on.sel {
                            None => None,
                            Some(sel) => {
                                let set = self.schema.relation_sel(rel, sel)?;
                                self.schema
                                    .normalize_ctors(self.schema.relation(rel).elem_def, set)
                            }
                        };
                        Ok((rel, ctors))
                    });
                match resolved {
                    Ok((rel, ctors)) => {
                        self.schema.checks[i].relation = rel;
                        self.schema.checks[i].ctors = ctors;
                    }
                    Err(e) => self.err(*file, c.on.span, e),
                }
                i += 1;
            }
        }
    }

    fn check_recursion(&mut self) {
        let n = self.schema.types.len();
        let mut state = vec![0u8; n];
        let mut errs = Vec::new();
        fn reached(ty: &Ty, out: &mut Vec<TypeId>) {
            match ty {
                Ty::Data { def, args, .. } => {
                    out.push(*def);
                    for a in args {
                        reached(a, out);
                    }
                }
                Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => reached(t, out),
                _ => {}
            }
        }
        fn visit(s: &Schema, t: usize, state: &mut [u8], errs: &mut Vec<usize>) {
            if state[t] == 2 {
                return;
            }
            if state[t] == 1 {
                errs.push(t);
                return;
            }
            state[t] = 1;
            let def = &s.types[t];
            let mut next = Vec::new();
            match &def.kind {
                TypeDefKind::Sum { header, ctors, .. } => {
                    for f in header
                        .iter()
                        .chain(ctors.iter().flat_map(|c| c.fields.iter()))
                    {
                        reached(&f.ty, &mut next);
                    }
                }
                TypeDefKind::Newtype { base, .. } => reached(base, &mut next),
            }
            for d in next {
                visit(s, d.0 as usize, state, errs);
            }
            state[t] = 2;
        }
        for t in 0..n {
            visit(&self.schema, t, &mut state, &mut errs);
        }
        errs.sort_unstable();
        errs.dedup();
        for t in errs {
            let def = &self.schema.types[t];
            let (file, span, name) = (def.file, def.span, def.name.clone());
            self.err(
                file,
                span,
                format!("type `{name}` reaches itself through its fields; recursion goes through references"),
            );
        }
    }

    /// References are direct fields, `Key r.S` or `Option (Key r.S)`, of types used only as relation element types.
    fn check_references(&mut self) {
        let mut errs = Vec::new();
        let has_refs: Vec<bool> = self
            .schema
            .types
            .iter()
            .map(|t| match &t.kind {
                TypeDefKind::Sum { header, ctors, .. } => header
                    .iter()
                    .chain(ctors.iter().flat_map(|c| c.fields.iter()))
                    .any(|f| mentions_key(&f.ty)),
                TypeDefKind::Newtype { .. } => false,
            })
            .collect();
        for t in &self.schema.types {
            let TypeDefKind::Sum { header, ctors, .. } = &t.kind else {
                continue;
            };
            for f in header
                .iter()
                .chain(ctors.iter().flat_map(|c| c.fields.iter()))
            {
                let direct = matches!(f.ty, Ty::Key { .. }) || self.is_option_key(&f.ty);
                if !direct && mentions_key(&f.ty) {
                    errs.push((
                        t.file,
                        f.span,
                        "a reference is a direct field of type `Key r.S` or `Option (Key r.S)`"
                            .to_string(),
                    ));
                }
                let mut inner = Vec::new();
                collect_defs(&f.ty, &mut inner);
                for d in inner {
                    if has_refs[d.0 as usize] {
                        errs.push((
                            t.file,
                            f.span,
                            format!(
                                "type `{}` holds references, so it is a relation's element type, never a field's type",
                                self.schema.ty(d).name
                            ),
                        ));
                    }
                }
            }
        }
        let mut owner: HashMap<TypeId, RelId> = HashMap::new();
        for (i, r) in self.schema.relations.iter().enumerate() {
            let id = RelId(i as u32);
            if r.elem_def.0 == u32::MAX || r.key.len() != 1 {
                continue;
            }
            let def = self.schema.ty(r.elem_def);
            if let Some(f) = def.field(0, &r.key[0])
                && matches!(f.ty, Ty::Key { rel, .. } if rel == id)
            {
                owner.insert(r.elem_def, id);
            }
        }
        for (i, r) in self.schema.relations.iter().enumerate() {
            if let Some(&o) = owner.get(&r.elem_def)
                && o.0 as usize != i
            {
                errs.push((
                    r.file,
                    r.span,
                    format!(
                        "type `{}` is keyed by `Key {}`, so it is the element type of `{}` only",
                        self.schema.ty(r.elem_def).name,
                        self.schema.relation(o).name,
                        self.schema.relation(o).name
                    ),
                ));
            }
        }
        for (f, s, m) in errs {
            self.err(f, s, m);
        }
    }

    fn check_ordered(&mut self) {
        let mut errs = Vec::new();
        for t in &self.schema.types {
            if !t.ordered {
                continue;
            }
            for f in t
                .header()
                .iter()
                .chain(t.ctors().iter().flat_map(|c| c.fields.iter()))
            {
                if !self.schema.is_ordered(&f.ty) {
                    errs.push((
                        t.file,
                        f.span,
                        format!(
                            "`{}` is declared ordered but field `{}` has an unordered type",
                            t.name, f.name
                        ),
                    ));
                }
            }
        }
        for (ty, file, span) in std::mem::take(&mut self.ordered_args) {
            if !self.schema.is_ordered(&ty) {
                errs.push((file, span, "`Span a` needs an ordered `a`".to_string()));
            }
        }
        for (f, s, m) in errs {
            self.err(f, s, m);
        }
    }
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

/// Resolves a written type in `scope`, recording `Span` arguments whose order is checked later.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per type form; each arm would have to be a function"
)]
pub fn resolve_type(
    schema: &Schema,
    scope: &TypeScope,
    t: &ast::Type,
    ordered_args: &mut Vec<(Ty, FileId, Span)>,
    file: FileId,
) -> Result<Ty, (Span, String)> {
    let field = scope.mode == TypeMode::Field;
    let go = |t: &ast::Type, ordered_args: &mut Vec<(Ty, FileId, Span)>| {
        resolve_type(schema, scope, t, ordered_args, file)
    };
    match &t.kind {
        ast::TypeKind::Var(v) => scope
            .params
            .iter()
            .position(|p| *p == v.text)
            .map(|i| Ty::Param(i as u32))
            .ok_or_else(|| (v.span, format!("no type parameter `{}`", v.text))),
        ast::TypeKind::Unit => {
            if field {
                Err((t.span, "a field has no unit type".to_string()))
            } else {
                Ok(Ty::Unit)
            }
        }
        ast::TypeKind::Fn(a, b) => {
            if field {
                return Err((t.span, "a field never has a function type".to_string()));
            }
            Ok(Ty::Fn(
                Box::new(go(a, ordered_args)?),
                Box::new(go(b, ordered_args)?),
            ))
        }
        ast::TypeKind::Record(fs) => {
            if field {
                return Err((
                    t.span,
                    "a declared field's type is a named type; declare a record type for it"
                        .to_string(),
                ));
            }
            let mut out: Vec<(String, Ty)> = Vec::new();
            for f in fs {
                if out.iter().any(|(n, _)| *n == f.name.text) {
                    return Err((
                        f.name.span,
                        format!("field `{}` is written twice", f.name.text),
                    ));
                }
                out.push((f.name.text.clone(), go(&f.ty, ordered_args)?));
            }
            out.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(Ty::Record(out))
        }
        ast::TypeKind::Sum(cs) => {
            if field {
                return Err((
                    t.span,
                    "a declared field's type is a named type; declare a sum type for it"
                        .to_string(),
                ));
            }
            let mut out: Vec<(String, Vec<(String, Ty)>)> = Vec::new();
            for c in cs {
                if out.iter().any(|(n, _)| *n == c.name.text) {
                    return Err((
                        c.name.span,
                        format!("constructor `{}` is written twice", c.name.text),
                    ));
                }
                let mut fields: Vec<(String, Ty)> = Vec::new();
                for f in c.record.iter().flatten() {
                    if fields.iter().any(|(n, _)| *n == f.name.text) {
                        return Err((
                            f.name.span,
                            format!("field `{}` is written twice", f.name.text),
                        ));
                    }
                    fields.push((f.name.text.clone(), go(&f.ty, ordered_args)?));
                }
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                out.push((c.name.text.clone(), fields));
            }
            out.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(Ty::Sum {
                ctors: out,
                sel: None,
            })
        }
        ast::TypeKind::Key(_, r) => {
            let rel = schema
                .lookup_relation(scope.module, &r.relation)
                .or_else(|e| schema.relation_ahead(scope.module, &r.relation).ok_or(e))
                .map_err(|e| (r.span, e))?;
            let late = schema.selects_late(rel);
            let ctors = match &r.sel {
                None => None,
                Some(_) if late && schema.ahead_relation(rel).is_some() => None,
                Some(_) if schema.ahead_relation(rel).is_some() => {
                    return Err((
                        r.span,
                        format!(
                            "`{}` is added by a later operator and its type is not added by this migration, so only `Key {}` may name it",
                            r.relation.name.text, r.relation.name.text
                        ),
                    ));
                }
                Some(sel) => match schema.relation_sel(rel, sel) {
                    Ok(set) => schema.normalize_ctors(schema.relation(rel).elem_def, set),
                    Err(_) if late => None,
                    Err(e) => return Err((r.span, e)),
                },
            };
            Ok(Ty::Key { rel, ctors })
        }
        ast::TypeKind::Named { name, sel, args } => {
            let found = schema
                .lookup_type(scope.module, name)
                .map_err(|e| (name.span(), e))?;
            let arity = |n: usize| -> Result<(), (Span, String)> {
                if args.len() == n {
                    Ok(())
                } else {
                    Err((
                        t.span,
                        format!(
                            "`{}` takes {n} type argument{}, given {}",
                            name.name.text,
                            if n == 1 { "" } else { "s" },
                            args.len()
                        ),
                    ))
                }
            };
            let no_sel = || -> Result<(), (Span, String)> {
                match sel {
                    None => Ok(()),
                    Some(_) => Err((
                        t.span,
                        format!("`{}` has no constructors to select", name.name.text),
                    )),
                }
            };
            let type_args =
                |ordered_args: &mut Vec<(Ty, FileId, Span)>| -> Result<Vec<Ty>, (Span, String)> {
                    args.iter()
                        .map(|a| match a {
                            ast::TypeArg::Type(t) => go(t, ordered_args),
                            ast::TypeArg::Int(_, span) => Err((
                                *span,
                                "a number is a type argument only as the scale of `Decimal`"
                                    .to_string(),
                            )),
                        })
                        .collect()
                };
            match found {
                TypeName::Base(b) => {
                    arity(0)?;
                    no_sel()?;
                    Ok(Ty::Base(b))
                }
                TypeName::Decimal => {
                    no_sel()?;
                    match args.as_slice() {
                        [ast::TypeArg::Int(s, span)] => {
                            if *s > 38 {
                                Err((*span, "a decimal scale is at most 38".to_string()))
                            } else {
                                Ok(Ty::Base(Base::Decimal(*s as u8)))
                            }
                        }
                        _ => Err((
                            t.span,
                            "`Decimal` takes its scale, a number: `Decimal 2`".to_string(),
                        )),
                    }
                }
                TypeName::Set => {
                    if field {
                        return Err((
                            t.span,
                            "a field never holds a set; a many-valued fact is a relation"
                                .to_string(),
                        ));
                    }
                    arity(1)?;
                    no_sel()?;
                    Ok(Ty::Set(Box::new(type_args(ordered_args)?.remove(0))))
                }
                TypeName::Group => Err((
                    t.span,
                    "`Group K T` is the type of a group binder and is never written; a group passed on is a `Set T`"
                        .to_string(),
                )),
                TypeName::Tag => {
                    arity(1)?;
                    no_sel()?;
                    let arg = type_args(ordered_args)?.remove(0);
                    match &arg {
                        Ty::Data { def, .. }
                            if matches!(schema.ty(*def).kind, TypeDefKind::Sum { .. }) =>
                        {
                            Ok(Ty::Tag(Box::new(arg)))
                        }
                        _ => Err((t.span, "`Tag T` takes a declared sum".to_string())),
                    }
                }
                TypeName::Decl(def) => {
                    let d = schema.ty(def);
                    arity(d.params.len())?;
                    let targs = type_args(ordered_args)?;
                    if def == schema.prelude_type("Span") {
                        ordered_args.push((targs[0].clone(), file, t.span));
                    }
                    let ctors = match sel {
                        None => None,
                        Some(sel) => {
                            if matches!(d.kind, TypeDefKind::Newtype { .. }) {
                                return Err((
                                    t.span,
                                    format!("newtype `{}` has no constructors", d.name),
                                ));
                            }
                            if field {
                                return Err((
                                    t.span,
                                    "a declared field's type is a whole sum, never a constructor row type".to_string(),
                                ));
                            }
                            let set = schema.type_sel(def, sel).map_err(|e| (t.span, e))?;
                            schema.normalize_ctors(def, set)
                        }
                    };
                    Ok(Ty::Data {
                        def,
                        args: targs,
                        ctors,
                    })
                }
            }
        }
    }
}
