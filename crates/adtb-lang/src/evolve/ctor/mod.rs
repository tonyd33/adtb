use super::*;

pub(in crate::evolve) mod remove;

impl Cx<'_> {
    /// Returns the key fields of the relations over type `def`.
    pub(in crate::evolve) fn key_fields(schema: &Schema, def: TypeId) -> BTreeSet<String> {
        Self::relations_of(schema, def)
            .into_iter()
            .flat_map(|(r, _, _)| schema.relation(r).key.clone())
            .collect()
    }
}

/// Collect the relation and constructor set of every `Key` type in `t`.
pub(super) fn key_types(t: &Ty, out: &mut Vec<(RelRef, Option<CtorSet>)>) {
    match t {
        Ty::Key { rel, ctors } => out.push((
            *rel,
            match ctors {
                KeyCtors::All | KeyCtors::Late(_) => None,
                KeyCtors::Only(cs) => Some(cs.clone()),
            },
        )),
        Ty::Data { args, .. } => args.iter().for_each(|a| key_types(a, out)),
        Ty::Set(x) | Ty::NonEmpty(x) | Ty::Tag(x) => key_types(x, out),
        Ty::Group(a, b) | Ty::Fn(a, b) => {
            key_types(a, out);
            key_types(b, out);
        }
        Ty::Record(fs) => fs.iter().for_each(|(_, x)| key_types(x, out)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .for_each(|(_, fs)| fs.iter().for_each(|(_, x)| key_types(x, out))),
        Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => {}
    }
}

/// Returns the field an index element `e` names first, and the rest of it from the dot after
/// that field.
pub(super) fn element_head(e: &str) -> (&str, &str) {
    match e.find('.') {
        Some(at) => (e[..at].trim(), &e[at..]),
        None => (e.trim(), ""),
    }
}

/// What a routing carries whole: (source constructor, field, product constructor, field).
pub(super) type Carries = Vec<(String, String, String, String)>;

/// The bindings of a routing or fill arm's pattern: the constructor it matches, a binder of the
/// whole row, and each variable bound to a field, with that field.
struct ArmBinding {
    from: Option<String>,
    binder: Option<String>,
    bound: BTreeMap<String, String>,
}

impl ArmBinding {
    fn of(a: &ast::Arm) -> ArmBinding {
        let mut out = ArmBinding {
            from: None,
            binder: None,
            bound: BTreeMap::new(),
        };
        let Some(ast::Pattern {
            kind:
                ast::PatternKind::Ctor {
                    path,
                    ctor,
                    payload,
                },
            ..
        }) = a.patterns.first()
        else {
            return out;
        };
        out.from = Some(ctor.as_ref().unwrap_or(&path.name).text.clone());
        match payload {
            Some(ast::Payload::Bind(x)) => out.binder = Some(x.text.clone()),
            Some(ast::Payload::Record(fs)) => {
                for f in fs {
                    match &f.pattern {
                        None => {
                            out.bound.insert(f.field.text.clone(), f.field.text.clone());
                        }
                        Some(ast::Pattern {
                            kind: ast::PatternKind::Bind(x),
                            ..
                        }) => {
                            out.bound.insert(x.text.clone(), f.field.text.clone());
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        out
    }

    /// Returns the field of the matched row that `e` is, whole.
    fn carried(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ast::ExprKind::Field(x, f) => match &x.kind {
                ast::ExprKind::Name(q)
                    if self.from.is_some()
                        && q.module.is_none()
                        && Some(&q.name.text) == self.binder.as_ref() =>
                {
                    Some(f.text.clone())
                }
                _ => None,
            },
            ast::ExprKind::Name(q) if q.module.is_none() => self.bound.get(&q.name.text).cloned(),
            _ => None,
        }
    }
}

/// Returns the fields of the matched row that routing arms `arms` read to choose an arm: those
/// a pattern constrains, and every one of `fields`, the header's included, when an arm has a
/// guard.
pub(super) fn matched_fields(arms: &[ast::Arm], fields: &[String]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for a in arms {
        if a.guard.is_some() {
            out.extend(fields.iter().cloned());
        }
        for p in &a.patterns {
            if let ast::PatternKind::Ctor {
                payload: Some(ast::Payload::Record(fs)),
                ..
            } = &p.kind
            {
                out.extend(
                    fs.iter()
                        .filter(|f| {
                            f.pattern
                                .as_ref()
                                .is_some_and(|x| !matches!(x.kind, ast::PatternKind::Bind(_)))
                        })
                        .map(|f| f.field.text.clone()),
                );
            }
        }
    }
    out.into_iter().collect()
}

/// Returns the constructor fill arm `a` matches, and the field of it its value is whole.
pub(super) fn fill_image(a: &ast::Arm) -> (Option<String>, Option<String>) {
    let b = ArmBinding::of(a);
    let carried = if a.guard.is_none() {
        b.carried(&a.body)
    } else {
        None
    };
    (b.from, carried)
}

/// Returns what routing arm `a` matches, makes, and carries whole.
pub(super) fn arm_image(a: &ast::Arm) -> carry::ArmImage {
    let b = ArmBinding::of(a);
    let (to, given) = match &a.body.kind {
        ast::ExprKind::Ctor {
            path: bp,
            ctor: bc,
            record,
        } => {
            let to = bc.as_ref().unwrap_or(&bp.name).text.clone();
            match record {
                None => (Some(to), Vec::new()),
                Some(ast::RecordExpr::Fields(given, _)) => (
                    Some(to),
                    given
                        .iter()
                        .map(|(g, e)| (g.text.clone(), b.carried(e)))
                        .collect(),
                ),
                Some(_) => (None, Vec::new()),
            }
        }
        _ => (None, Vec::new()),
    };
    carry::ArmImage {
        from: b.from,
        to,
        given,
    }
}
