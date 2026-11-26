use adtb_lang::ids::{CatCtorId, CatFieldId};
use adtb_lang::schema::Schema;

use crate::exec::Val;

use super::{Broken, Error};

/// Where a row of one engine constructor holds each of the fields the engine gives it.
#[derive(Clone, Debug)]
pub(crate) struct EngineRow<const N: usize> {
    ctor: u32,
    places: [usize; N],
}

impl<const N: usize> EngineRow<N> {
    /// Returns where `schema` places each of `fields` in a row of constructor `ctor`.
    ///
    /// Fail with `Broken` when `schema` declares no constructor `ctor`, or `fields` does not name
    /// each of its fields once.
    pub(crate) fn new(
        schema: &Schema,
        ctor: CatCtorId,
        fields: [CatFieldId; N],
    ) -> Result<Self, Error> {
        let broken = |detail: String| Error::from(Broken::EngineCtor { ctor, detail });
        let c = (schema.ids().ctor_of(ctor)).ok_or_else(|| broken("is not declared".into()))?;
        let declared: Vec<CatFieldId> = (schema.ids().type_field_ids(c.def()))
            .filter(|(f, _)| f.ctor().is_none_or(|own| own == c.index()))
            .map(|(_, id)| id)
            .collect();
        let mut places = [0; N];
        for (place, f) in places.iter_mut().zip(fields) {
            *place = (declared.iter().position(|d| *d == f))
                .ok_or_else(|| broken(format!("declares no field {f}")))?;
        }
        if declared.len() != N {
            return Err(broken(format!(
                "declares {} fields, given {N}",
                declared.len()
            )));
        }
        if let Some(d) = declared.iter().find(|d| !fields.contains(d)) {
            return Err(broken(format!("is given no field {d}")));
        }
        Ok(EngineRow {
            ctor: c.index(),
            places,
        })
    }

    /// Returns the row holding `values[i]` in field `fields[i]` of [`EngineRow::new`].
    pub(crate) fn row(&self, values: [Val; N]) -> Val {
        let mut placed: Vec<(usize, Val)> = self.places.into_iter().zip(values).collect();
        placed.sort_by_key(|(at, _)| *at);
        Val::ctor(self.ctor, placed.into_iter().map(|(_, v)| v).collect())
    }
}
