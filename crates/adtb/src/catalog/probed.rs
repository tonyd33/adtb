use adtb_lang::schema::{ModuleDef, Schema, ValueDef};

use crate::runtime::Runtime;
use crate::storage::{PageError, Pager};

use super::Catalog;
use super::root::{Changing, Root};

impl Catalog {
    /// Returns the structures of the relations of the modules the engine declares when `engine`,
    /// else of every other module, each line naming relations and constructors by name: the
    /// root's change in progress, the next heap id, then for each module in name order a line
    /// `RELATION.CTOR heap N` for each constructor, a line `RELATION CLAUSE over CTOR | ...` for
    /// each index, and a line `TYPE order CTOR < ...` for each sum of several constructors the
    /// module declares, its constructors in order key order. A heap id counts from 0 at one past
    /// the highest id a structure of a relation the engine declares holds, its history
    /// included, and is negative below it; a relation the engine declares shows `heap` alone.
    pub(crate) fn probed<R: Runtime>(
        &self,
        pager: &Pager<R>,
        engine: bool,
    ) -> Result<Vec<String>, PageError> {
        let schema: &Schema = &self.schema;
        let declared = |r| schema.module(schema.relation(r).module).engine;
        let own = self.heaps.iter().filter(|((r, _), _)| declared(*r));
        let base = own
            .map(|(_, h)| h.id().0)
            .chain(self.kept.ids(declared))
            .max()
            .map_or(0, |top| top + 1);
        let changing = match Root::read(&pager.read(self.root)?).changing() {
            Changing::Schema(_) => "Schema".to_owned(),
            c => format!("{c:?}"),
        };
        let mut out = vec![
            format!("change {changing}"),
            format!("next heap {}", i64::from(self.next_heap) - i64::from(base)),
        ];
        let mut modules: Vec<&ModuleDef> = schema
            .modules
            .iter()
            .filter(|m| m.engine == engine)
            .collect();
        modules.sort_by(|a, b| a.name.cmp(&b.name));
        for m in modules {
            self.relations_of(m, base, &mut out);
            self.orders_of(m, &mut out);
        }
        Ok(out)
    }

    /// Add the lines of the heaps and indexes of each relation of `m` to `out`.
    fn relations_of(&self, m: &ModuleDef, base: u32, out: &mut Vec<String>) {
        let schema: &Schema = &self.schema;
        for (name, value) in &m.values {
            let &ValueDef::Relation(r) = value else {
                continue;
            };
            let qualified = format!("{}::{name}", m.name);
            let ctors = schema.ty(schema.relation(r).elem_def).ctors();
            let ctor_names = ctors.iter().map(|c| c.name.as_str());
            for (c, ctor) in ctor_names.enumerate() {
                let heap = &self.heaps[&(r, c as u32)];
                let id = if m.engine {
                    String::new()
                } else {
                    format!(" {}", i64::from(heap.id().0) - i64::from(base))
                };
                out.push(format!("{qualified}.{ctor} heap{id}"));
            }
            let named = |c: &u32| ctors[*c as usize].name.as_str();
            let mut indexes: Vec<String> = self
                .indexes
                .values()
                .filter(|(s, _, _)| s.relation == r)
                .map(|(s, _, _)| {
                    let over: Vec<&str> = s.ctors.iter().map(named).collect();
                    format!("{qualified} {} over {}", s.clause, over.join(" | "))
                })
                .collect();
            indexes.sort();
            out.extend(indexes);
        }
    }

    /// Add a line of the constructors of each sum of several constructors `m` declares, in order
    /// key order, to `out`.
    fn orders_of(&self, m: &ModuleDef, out: &mut Vec<String>) {
        let schema: &Schema = &self.schema;
        for (name, &t) in &m.types {
            let ty = schema.ty(t);
            if ty.ctors().len() < 2 {
                continue;
            }
            let mut ordered: Vec<(Option<&_>, &str)> = ty
                .ctor_refs()
                .map(|c| {
                    let id = schema.ids().ctor_id(c);
                    let key = id.and_then(|id| self.keys.get(&id.raw()));
                    (key, ty.ctors()[c.index() as usize].name.as_str())
                })
                .collect();
            ordered.sort();
            let names: Vec<&str> = ordered.into_iter().map(|(_, n)| n).collect();
            out.push(format!("{}::{name} order {}", m.name, names.join(" < ")));
        }
    }
}
