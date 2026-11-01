use super::Schema;
use crate::ast::{CtorSel, Expr, ExprKind};
use crate::types::RelId;

impl Schema {
    /// Returns the canonical constraint name of the unique at `index` of `rel`.
    ///
    /// Precondition: `index` identifies a declared unique of `rel` in this schema.
    pub fn unique_name(&self, rel: RelId, index: usize) -> String {
        let r = self.relation(rel);
        let name = format!("{}::{}", self.module(r.module).name, r.name);
        let ix = &r.indexes[index];
        let paths: Option<Vec<_>> = ix.fields.iter().map(field_path).collect();
        let Some(paths) = paths else {
            let id = self.ids.indexes[&(rel, index)];
            return format!("{name}.unique#{id}");
        };
        let name = format!("{name}.unique({})", paths.join(", "));
        let ctors = self.ty(r.elem_def).ctors();
        let Some(scope) = ix.scope.as_ref().filter(|scope| scope.len() != ctors.len()) else {
            return name;
        };
        let mut names: Vec<_> = scope
            .iter()
            .map(|c| ctors[*c as usize].name.as_str())
            .collect();
        names.sort_unstable();
        let set = match names.as_slice() {
            [one] => (*one).to_owned(),
            many => format!("({})", many.join(" | ")),
        };
        format!("{name} in {set}")
    }
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
