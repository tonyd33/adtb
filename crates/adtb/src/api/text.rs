use std::collections::BTreeMap;
use std::sync::Arc;

use adtb_lang::ast::AdhocItem;
use adtb_lang::check::{AdhocCommand, Bundles, Command, Explained, Program, check_adhoc_recording};
use adtb_lang::kernel::{Kind, Terms, Tm, Var};
use adtb_lang::norm;
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::schema::Schema;
use adtb_lang::textual::Form;
use adtb_lang::types::Ty;

use crate::exec::Val;
use crate::write::Outcome;

use super::command;
use super::error::Error;

/// The rows one item of ad hoc text returned, with their type.
#[derive(Debug)]
pub struct DynRows {
    ty: Ty,
    form: Form,
    rows: std::vec::IntoIter<Val>,
}

impl DynRows {
    /// Returns the type of each row.
    pub fn ty(&self) -> &Ty {
        &self.ty
    }

    /// Returns the form the textual syntax prints each row at.
    pub fn form(&self) -> &Form {
        &self.form
    }
}

impl Iterator for DynRows {
    type Item = Result<Val, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rows.next().map(Ok)
    }
}

/// Ad hoc text checked and normalised against one schema, its parameters bound.
pub(super) struct Prepared {
    /// The schema checked against, extended with the temp relations the text declares.
    schema: Arc<Schema>,
    terms: Terms,
    items: Vec<Tm>,
    commands: Vec<AdhocCommand>,
    params: Vec<(Var, Val)>,
    /// Whether an item writes.
    pub writes: bool,
    /// The least `retry n` a `transact` block of the text states.
    pub retry: Option<u64>,
    /// Whether a `transact` block of the text is `read_only deferrable`.
    pub deferrable: bool,
}

impl Prepared {
    /// Returns `text` checked against `program` of `schema`, each free name typed and bound by
    /// `params`.
    pub fn new(
        schema: &Arc<Schema>,
        program: &Program,
        bundles: &Bundles,
        text: &str,
        params: &[(&str, Ty, Val)],
    ) -> Result<Prepared, Error> {
        let items = parse_adhoc(text, Options::default())
            .ok()
            .map_err(|ds| Error::invalid(messages(ds.iter().map(|d| &d.message))))?;
        if items.iter().any(|i| {
            !matches!(
                i,
                AdhocItem::Expr(_)
                    | AdhocItem::Write(_)
                    | AdhocItem::Transact(_)
                    | AdhocItem::Temp(_)
                    | AdhocItem::Command(..)
            )
        }) {
            return Err(Error::invalid(
                "only queries, writes, transact blocks, temp relations and commands run as ad hoc \
                 text in process",
            ));
        }
        let types: BTreeMap<String, Ty> = params
            .iter()
            .map(|(n, ty, _)| ((*n).to_owned(), ty.clone()))
            .collect();
        let adhoc = check_adhoc_recording(schema, program, &items, &types, bundles, None)
            .map_err(|ds| Error::invalid(messages(ds.iter().map(|d| &d.message))))?;
        let normal = norm::adhoc(adhoc.schema(schema), program, &adhoc).map_err(|e| {
            Error::rejected(super::error::RejectReason::Internal {
                what: format!("checked ad hoc text has no normal form: {e:?}"),
            })
        })?;
        let bound = params
            .iter()
            .filter_map(|(n, _, v)| normal.params.get(*n).map(|x| (*x, v.clone())))
            .collect();
        let mut writes = false;
        let mut retry: Option<u64> = None;
        let mut deferrable = false;
        for c in &normal.commands {
            command::runs(&c.command)?;
            if let Command::Explain {
                analyze: true,
                target: Explained::Statement(t),
            } = &c.command
            {
                writes |= normal.terms.writes(*t);
            }
        }
        for t in &normal.items {
            writes |= normal.terms.writes(*t);
            if let Kind::Transact(_, o) = normal.terms.kind(*t) {
                if let Some(n) = o.retry {
                    retry = Some(retry.map_or(n, |r| r.min(n)));
                }
                deferrable |= o.deferrable;
            }
        }
        Ok(Prepared {
            schema: adhoc.temps.map_or_else(|| schema.clone(), Arc::new),
            terms: normal.terms,
            items: normal.items,
            commands: normal.commands,
            params: bound,
            writes,
            retry,
            deferrable,
        })
    }

    /// Returns the schema the text's terms name declarations of.
    pub fn schema(&self) -> &Arc<Schema> {
        &self.schema
    }

    pub fn terms(&self) -> &Terms {
        &self.terms
    }

    pub fn items(&self) -> &[Tm] {
        &self.items
    }

    pub fn commands(&self) -> &[AdhocCommand] {
        &self.commands
    }

    pub fn params(&self) -> &[(Var, Val)] {
        &self.params
    }

    /// Returns the rows of each item of `outcomes`, one outcome per item, that yields a result,
    /// each of `commands`, by the number of items before it, at its place among them.
    pub fn results(
        &self,
        schema: &Schema,
        outcomes: Vec<Outcome>,
        commands: Vec<(usize, DynRows)>,
    ) -> Result<Vec<DynRows>, Error> {
        let mut out = Vec::new();
        let mut commands = commands.into_iter().peekable();
        for (i, (t, o)) in self.items.iter().zip(outcomes).enumerate() {
            while let Some((_, rows)) = commands.next_if(|(at, _)| *at <= i) {
                out.push(rows);
            }
            let ty = self.terms.ty(*t);
            match ty {
                Ty::Change | Ty::Unit => {}
                Ty::Record(fs) if fs.is_empty() => {}
                Ty::Set(elem) | Ty::NonEmpty(elem) => {
                    out.push(rows(schema, elem.as_ref().clone(), elements(o.result)?));
                }
                _ => out.push(rows(schema, ty.clone(), vec![o.result])),
            }
        }
        out.extend(commands.map(|(_, rows)| rows));
        Ok(out)
    }
}

/// Returns `rows`, each a value of `ty`.
pub(super) fn rows(schema: &Schema, ty: Ty, rows: Vec<Val>) -> DynRows {
    DynRows {
        form: Form::of(schema, &ty),
        ty,
        rows: rows.into_iter(),
    }
}

fn elements(v: Val) -> Result<Vec<Val>, Error> {
    match v {
        Val::Set(s) => Ok(s.iter().cloned().collect()),
        Val::List(l) => Ok(l.to_vec()),
        other => Err(Error::rejected(super::error::RejectReason::Internal {
            what: format!("a set-valued item evaluates to a set, not {other:?}"),
        })),
    }
}

fn messages<'a>(ms: impl Iterator<Item = &'a String>) -> String {
    ms.map(String::as_str).collect::<Vec<_>>().join("\n")
}
