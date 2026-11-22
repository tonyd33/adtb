use super::Schema;
use crate::ast::{CtorSel, Expr, ExprKind};
use crate::types::RelId;

impl Schema {
    /// Returns the canonical constraint name of the unique at `index` of `rel`.
    ///
    /// Precondition: `index` identifies a declared unique of `rel` in this schema.
    pub fn unique_name(&self, rel: RelId, index: usize) -> String {
        self.field_unique_name(rel, index).unwrap_or_else(|| {
            let r = self.relation(rel);
            let id = crate::ids::held_id(self.ids.index_id(rel, index));
            format!("{}::{}.unique#{id}", self.module(r.module).name, r.name)
        })
    }

    /// Returns the canonical constraint name of the unique at `index` of `rel`, `None` when an
    /// element is not a field path.
    pub(crate) fn field_unique_name(&self, rel: RelId, index: usize) -> Option<String> {
        let r = self.relation(rel);
        let ix = &r.indexes[index];
        let paths: Vec<_> = ix.fields.iter().map(field_path).collect::<Option<_>>()?;
        let ctors = self.ty(r.elem_def).ctors();
        let scope: Option<Vec<_>> = ix.scope.as_ref().map(|scope| {
            scope
                .iter()
                .map(|c| ctors[*c as usize].name.as_str())
                .collect()
        });
        Some(canonical_unique_name(
            &self.module(r.module).name,
            &r.name,
            &paths,
            scope.as_deref(),
            ctors.len(),
        ))
    }
}

/// Returns the canonical constraint name of a unique of relation `rel` of module `module` over
/// field paths `paths`, ranging over the constructors named `scope` of the relation's `ctors`
/// constructors, every one when `None`.
pub fn canonical_unique_name(
    module: &str,
    rel: &str,
    paths: &[String],
    scope: Option<&[&str]>,
    ctors: usize,
) -> String {
    let name = format!("{module}::{rel}.unique({})", paths.join(", "));
    let Some(scope) = scope.filter(|scope| scope.len() != ctors) else {
        return name;
    };
    let mut names = scope.to_vec();
    names.sort_unstable();
    let set = match names.as_slice() {
        [one] => (*one).to_owned(),
        many => format!("({})", many.join(" | ")),
    };
    format!("{name} in {set}")
}

/// Returns the field path `e` is, written `f.C.g`.
pub fn field_path(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Name(q) if q.module.is_none() => Some(q.name.text.clone()),
        ExprKind::Field(x, f) => Some(format!("{}.{}", field_path(x)?, f.text)),
        ExprKind::Select(x, CtorSel::One(c)) => Some(format!("{}.{}", field_path(x)?, c.text)),
        _ => None,
    }
}
