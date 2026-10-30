//! Values of catalog rows, named by module, type, relation, constructor and field.

use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::RelId;

use crate::key::Value;

use super::layout;

/// Returns relation `catalog::name` of `schema`.
pub fn relation(schema: &Schema, name: &str) -> RelId {
    let m = schema.module_named("catalog").expect("the catalog module");
    match schema.module(m).values.get(name) {
        Some(&ValueDef::Relation(r)) => r,
        _ => panic!("the catalog declares relation `{name}`"),
    }
}

/// Returns the value of the nullary constructor `ctor` of type `module::ty` of `schema`.
pub fn tag(schema: &Schema, module: &str, ty: &str, ctor: &str) -> Value {
    let m = schema.module_named(module).expect("an engine module");
    let def = schema.module(m).types[ty];
    let c = schema
        .ty(def)
        .ctor_index(ctor)
        .unwrap_or_else(|| panic!("`{ty}` has constructor `{ctor}`"));
    Value::Sum {
        ctor: c,
        fields: Vec::new(),
    }
}

/// Returns `b` as a value of the prelude's `Bool`.
pub fn boolean(schema: &Schema, b: bool) -> Value {
    tag(schema, "prelude", "Bool", if b { "True" } else { "False" })
}

/// Returns the stored row of constructor `ctor` of relation `r` whose fields `fields` gives by
/// name.
///
/// Precondition: `fields` gives each field of the constructor once and no other.
pub fn row(schema: &Schema, r: RelId, ctor: u32, fields: Vec<(&str, Value)>) -> Vec<Value> {
    let names = layout::row(schema, r, ctor);
    let shown = || {
        let td = schema.ty(schema.relation(r).elem_def);
        format!(
            "{}.{}",
            schema.relation(r).name,
            td.ctors()[ctor as usize].name
        )
    };
    assert_eq!(
        names.len(),
        fields.len(),
        "a row of `{}` gives every field",
        shown()
    );
    names
        .iter()
        .map(|(n, _)| {
            fields
                .iter()
                .find(|(f, _)| f == n)
                .unwrap_or_else(|| panic!("a row of `{}` gives `{n}`", shown()))
                .1
                .clone()
        })
        .collect()
}
