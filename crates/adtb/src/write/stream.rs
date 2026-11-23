//! A read-only statement whose result is produced a unit at a time.

use crate::exec;
use crate::runtime::{Instant, Runtime};

use super::{Bounds, Database, Error, Leave, Stop, Text, Txn, fresh, input};

/// What [`Database::produce`] made of a query.
#[derive(Debug)]
pub enum Produced<R: Runtime> {
    /// The query's whole value: it forms no set.
    Whole(exec::Val),
    Stream(Production<R>),
}

/// A query's elements, produced a unit at a time. Its statement limit counts only the time its
/// units run.
#[derive(Debug)]
pub struct Production<R: Runtime> {
    units: exec::Production<R>,
    rt: R,
    bounds: Bounds,
    /// Where the statement limit counts from: the start, later by every pause between units.
    since: Instant,
    /// When the last unit ended.
    paused: Instant,
}

impl<R: Runtime> Production<R> {
    /// Run the next unit, appending the distinct elements it makes to `out`. Returns whether a
    /// unit is left.
    pub fn next(&mut self, out: &mut Vec<exec::Val>) -> Result<bool, Error> {
        let now = self.rt.now();
        self.since = self.since + now.saturating_since(self.paused);
        let stop = Stop::of(&self.bounds, Some(self.since), None);
        let more = self.units.next(stop.deadline, out);
        self.paused = self.rt.now();
        Ok(more?)
    }
}

impl<R: Runtime> Database<R> {
    /// Returns the one item of `text`, a query, as a production of its elements under
    /// `bounds`, or `None` when the item reads temp, volatile or file rows, generates values,
    /// or is not one query.
    ///
    /// Preconditions: `text` has exactly one item.
    pub fn produce(
        &self,
        _leave: &Leave,
        tx: &Txn,
        text: Text<'_>,
        bounds: &Bounds,
    ) -> Result<Option<Produced<R>>, Error> {
        tx.noted(self.produced(tx, text, bounds))
    }

    fn produced(
        &self,
        tx: &Txn,
        text: Text<'_>,
        bounds: &Bounds,
    ) -> Result<Option<Produced<R>>, Error> {
        self.admit(&text)?;
        let started = self.rt.now();
        let [root] = text.items else {
            return Ok(None);
        };
        let mut terms = text.terms.clone();
        let mut occurrences = fresh::Occurrences::new();
        let root = fresh::rewrite(text.schema, &mut terms, *root, &[], &mut occurrences);
        if !occurrences.is_empty()
            || !input::inputs(text.schema, &terms, root).is_empty()
            || matches!(terms.ty(root), adtb_lang::types::Ty::Change)
            || matches!(terms.kind(root), adtb_lang::kernel::Kind::Transact(..))
        {
            return Ok(None);
        }
        let free = text.params.iter().map(|(v, _)| *v).collect();
        let terms = adtb_lang::kernel::ValidTerms::new(text.schema, terms, &[(root, free)])
            .map_err(super::Broken::IllTyped)?;
        let program = self.program(text.schema, terms.valid(), root, text.caller.cloned())?;
        let options = exec::Options {
            fix_rounds: bounds.fix_rounds.unwrap_or(self.options.fix_rounds),
            ..self.options
        };
        let stop = Stop::of(bounds, Some(started), None);
        let call = self.call(&self.store(tx), options, &stop, &self.grant_of(tx, bounds)?)?;
        let bound = std::sync::Arc::new(program).bind(|v| {
            text.params
                .iter()
                .find(|(p, _)| *p == v)
                .map(|(_, x)| x.clone())
        })?;
        let produced = exec::produce(self.rt.clone(), bound, call)?;
        Ok(Some(match produced {
            exec::Produced::Whole(v) => Produced::Whole(v),
            exec::Produced::Stream(units) => Produced::Stream(Production {
                units,
                rt: self.rt.clone(),
                bounds: bounds.clone(),
                since: started,
                paused: self.rt.now(),
            }),
        }))
    }
}
