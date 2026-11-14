use super::*;

/// A temp relation of ad hoc text, its row type resolved.
#[derive(Clone, Debug)]
pub(crate) struct Temp {
    name: String,
    row: TempRow,
    key: Vec<String>,
    span: Span,
    row_span: Span,
}

impl Temp {
    /// Returns the temp relation `decl` declares, of row type `row` and key fields `key`.
    pub(crate) fn new(decl: &ast::TempDecl, row: TempRow, key: Vec<String>) -> Temp {
        Temp {
            name: decl.name.text.clone(),
            row,
            key,
            span: decl.span,
            row_span: decl.ty.span,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum TempRow {
    /// An anonymous record, its fields in declaration order.
    Record(Vec<(String, Ty)>),
    Declared {
        def: TypeId,
        args: Vec<Ty>,
    },
}

impl Schema {
    /// Returns the schema with each of `temps` appended to the relations of module
    /// [`crate::check::TEMP_MODULE`], an anonymous record row declared as a record type of that
    /// module; returns the relation of each.
    #[must_use]
    pub(crate) fn with_temps(&self, temps: &[Temp]) -> (Schema, Vec<RelId>) {
        let mut out = self.clone();
        let temp = crate::check::TEMP_MODULE;
        let module = out.module_named(temp).unwrap_or_else(|| {
            let defs = out.defs_mut();
            defs.modules.push(ModuleDef {
                name: temp.to_string(),
                engine: false,
                types: BTreeMap::new(),
                values: BTreeMap::new(),
            });
            ModuleId(defs.modules.len() as u32 - 1)
        });
        let mut declared = Vec::new();
        for t in temps {
            let defs = out.defs_mut();
            let (elem, elem_def) = match &t.row {
                TempRow::Record(fields) => {
                    let fields = fields
                        .iter()
                        .map(|(name, ty)| FieldDef {
                            name: name.clone(),
                            ty: ty.clone(),
                            on_delete: OnDelete::Restrict,
                            span: t.row_span,
                        })
                        .collect();
                    let def = defs.push_record(&t.name, module, fields, (t.row_span, t.span));
                    let elem = Ty::Data {
                        def,
                        args: Vec::new(),
                        ctors: None,
                    };
                    (elem, def)
                }
                TempRow::Declared { def, args } => (
                    Ty::Data {
                        def: *def,
                        args: args.clone(),
                        ctors: None,
                    },
                    *def,
                ),
            };
            let rel = RelId(defs.relations.len() as u32);
            defs.relations.push(RelationDef {
                name: t.name.clone(),
                module,
                elem,
                elem_def,
                key: t.key.clone(),
                generate: None,
                indexes: Vec::new(),
                defaults: Vec::new(),
                computed: Vec::new(),
                sets: Vec::new(),
                volatile: false,
                file: FileId(0),
                span: t.span,
            });
            defs.modules[module.0 as usize]
                .values
                .insert(t.name.clone(), ValueDef::Relation(rel));
            let id = TempRelId::after(out.temps.values().max().copied());
            out.temps.insert(rel, id);
            declared.push(rel);
        }
        (out, declared)
    }
}
