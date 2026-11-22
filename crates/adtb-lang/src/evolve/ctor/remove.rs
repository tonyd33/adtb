use super::*;

/// The relations over a type, each as its id, module and name.
pub(in crate::evolve) type Rels = [(RelId, String, String)];

/// Returns why removing constructor `c` of type `def` would break the schema.
pub(in crate::evolve) fn removal_blocked(
    schema: &Schema,
    t: &ast::TypeDecl,
    name: &str,
    def: TypeId,
    c: u32,
    rels: &Rels,
) -> Option<String> {
    for u in &schema.types {
        for f in u
            .header()
            .iter()
            .chain(u.ctors().iter().flat_map(|x| x.fields.iter()))
        {
            let mut keys = Vec::new();
            key_types(&f.ty, &mut keys);
            for (rel, ctors) in keys {
                let RelRef::Declared(rel) = rel else {
                    continue;
                };
                if schema.relation(rel).elem_def == def
                    && let Some(cs) = ctors
                    && cs.contains(&c)
                {
                    return Some(format!(
                        "field `{}.{}` references a constructor set that holds it",
                        u.name, f.name
                    ));
                }
            }
        }
    }
    for (rel, _, r) in rels {
        for s in &schema.relation(*rel).sets {
            if s.ctors == [c] {
                return Some(format!(
                    "the named set `{}` of `{r}` would become empty",
                    s.name
                ));
            }
        }
    }
    if schema.ty(def).ctors().len() == 1 {
        return Some(format!(
            "`{name}` is the only constructor of `{}`",
            t.name.text
        ));
    }
    schema
        .checks
        .iter()
        .find(|chk| {
            rels.iter().any(|(r, _, _)| *r == chk.relation)
                && chk.ctors.as_ref().is_some_and(|cs| cs == &[c])
        })
        .map(|chk| {
            format!(
                "check `{}` requires rows of `{name}` and has no image without it",
                chk.name
            )
        })
}
