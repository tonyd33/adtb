use std::collections::BTreeMap;

use adtb_lang::ast::AdhocItem;
use adtb_lang::check::{Program, check_adhoc};
use adtb_lang::kernel::{Kind, Terms, Tm, Var};
use adtb_lang::norm;
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::schema::Schema;
use adtb_lang::textual::Form;
use adtb_lang::types::Ty;

use crate::exec::Val;
use crate::write::Outcome;

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
    terms: Terms,
    items: Vec<Tm>,
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
        schema: &Schema,
        program: &Program,
        text: &str,
        params: &[(&str, Ty, Val)],
    ) -> Result<Prepared, Error> {
        let items = parse_adhoc(text, Options::default())
            .ok()
            .map_err(|ds| Error::invalid(messages(ds.iter().map(|d| &d.message))))?;
        if items.iter().any(|i| {
            !matches!(
                i,
                AdhocItem::Expr(_) | AdhocItem::Write(_) | AdhocItem::Transact(_)
            )
        }) {
            return Err(Error::invalid(
                "only queries, writes and transact blocks run as ad hoc text in process",
            ));
        }
        let types: BTreeMap<String, Ty> = params
            .iter()
            .map(|(n, ty, _)| ((*n).to_owned(), ty.clone()))
            .collect();
        let adhoc = check_adhoc(schema, program, &items, &types)
            .map_err(|ds| Error::invalid(messages(ds.iter().map(|d| &d.message))))?;
        let normal = norm::adhoc(schema, program, &adhoc).map_err(|e| {
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
        for t in &normal.items {
            writes |= has_write(&normal.terms, *t);
            if let Kind::Transact(_, o) = normal.terms.kind(*t) {
                if let Some(n) = o.retry {
                    retry = Some(retry.map_or(n, |r| r.min(n)));
                }
                deferrable |= o.deferrable;
            }
        }
        Ok(Prepared {
            terms: normal.terms,
            items: normal.items,
            params: bound,
            writes,
            retry,
            deferrable,
        })
    }

    pub fn terms(&self) -> &Terms {
        &self.terms
    }

    pub fn items(&self) -> &[Tm] {
        &self.items
    }

    pub fn params(&self) -> &[(Var, Val)] {
        &self.params
    }

    /// Returns the rows of each item of `outcomes`, one outcome per item, that yields a result.
    pub fn results(&self, schema: &Schema, outcomes: Vec<Outcome>) -> Result<Vec<DynRows>, Error> {
        let mut out = Vec::new();
        for (t, o) in self.items.iter().zip(outcomes) {
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
        Ok(out)
    }
}

fn rows(schema: &Schema, ty: Ty, rows: Vec<Val>) -> DynRows {
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

fn has_write(terms: &Terms, t: Tm) -> bool {
    matches!(
        terms.kind(t),
        Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Replace(..)
            | Kind::Upsert { .. }
            | Kind::Rejects { .. }
    ) || terms.children(t).into_iter().any(|c| has_write(terms, c))
}
