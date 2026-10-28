//! The commands of ad hoc text run in process, from their checked terms.

use std::collections::BTreeMap;

use adtb_lang::check::{Command, Explained, Program};
use adtb_lang::kernel::{Kind, Terms, Tm, alpha_equal};
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::{Base, CtorSet, RelId, Ty};

use crate::exec::Val;
use crate::plan::explain::{Actual, Node, json_text, object};
use crate::runtime::Runtime;
use crate::write::{self, Bounds, Local, Outcome, Text, Txn};

use super::text::{DynRows, Prepared, rows};
use super::{Engine, Error, unsupported};

/// The rows [`Engine::swap_row`] replaces.
pub(super) enum Old {
    /// Every row of the constructor.
    Ctor(u32),
    /// The one row.
    Row(Val),
}

/// What `explain analyze` adds to its result: a member of the document, also a line of the text.
struct Added {
    name: &'static str,
    line: String,
    json: String,
}

/// Refuses a command the engine does not run in process.
pub(super) fn runs(c: &Command) -> Result<(), Error> {
    match c {
        Command::Explain {
            target: Explained::Statement(_),
            ..
        }
        | Command::Vacuum(_)
        | Command::Reindex { .. }
        | Command::SetSetting { .. }
        | Command::RetireBundle(_)
        | Command::SupersedeBundle(..) => Ok(()),
        other => Err(unsupported(&format!("`{}`", word(other)))),
    }
}

/// Returns the words a command starts with.
fn word(c: &Command) -> &'static str {
    match c {
        Command::Explain { .. } => "explain",
        Command::PinPlan(_) => "pin plan",
        Command::UnpinPlan(_) => "unpin plan",
        Command::CancelCall(_) => "cancel call",
        Command::TerminateSession(_) => "terminate session",
        Command::Vacuum(_) => "vacuum",
        Command::Verify(_) => "verify",
        Command::Reindex { .. } => "reindex",
        Command::Compact => "compact",
        Command::FinalizeUpgrade => "finalize upgrade",
        Command::RetireBundle(_) => "retire bundle",
        Command::SupersedeBundle(..) => "supersede bundle",
        Command::SetSetting { .. } => "set setting",
        Command::Export { .. } => "export",
        Command::RegisterBundle(_) => "register bundle",
        Command::ApplyMigrations(_) => "apply migrations",
        Command::PrintSchema => "print schema",
        Command::ArchiveAudit { .. } => "archive audit",
        Command::RotateKek(_) | Command::RotateDek => "rotate key",
        Command::DestroyDek(_) => "destroy key",
        Command::Dump { .. } => "dump",
        Command::Restore { .. } => "restore",
        Command::Backup { .. } => "backup",
    }
}

/// Returns the relation `name` of engine module `module`.
fn engine_relation(schema: &Schema, module: &str, name: &str) -> Result<RelId, write::Error> {
    schema
        .module_named(module)
        .and_then(|m| match schema.module(m).values.get(name) {
            Some(ValueDef::Relation(r)) => Some(*r),
            _ => None,
        })
        .ok_or_else(|| write::Error::Internal(format!("the engine declares `{module}::{name}`")))
}

/// Returns the positions among `rel`'s indexes of those `reindex` rebuilds: every one when
/// `index` is `None`, else every one declared with the same fields over the same constructor
/// set as the one at `index`.
pub(super) fn reindexed(
    schema: &Schema,
    program: &Program,
    rel: RelId,
    index: Option<usize>,
) -> Vec<usize> {
    let r = schema.relation(rel);
    let Some(i) = index else {
        return (0..r.indexes.len()).collect();
    };
    let scope = |s: &Option<CtorSet>| {
        s.clone()
            .and_then(|s| schema.normalize_ctors(r.elem_def, s))
    };
    let named = &r.indexes[i];
    let fields = |j: usize| {
        (0..named.fields.len())
            .map(|k| program.index_fields.get(&(rel, j, k)))
            .collect::<Option<Vec<_>>>()
    };
    let Some(want) = fields(i) else {
        return vec![i];
    };
    (0..r.indexes.len())
        .filter(|&j| {
            let ix = &r.indexes[j];
            j == i
                || ix.fields.len() == named.fields.len()
                    && scope(&ix.scope) == scope(&named.scope)
                    && fields(j).is_some_and(|got| {
                        got.iter().zip(&want).all(|(a, b)| {
                            a.path == b.path
                                && alpha_equal(&program.terms, a.value, &program.terms, b.value)
                        })
                    })
        })
        .collect()
}

impl<R: Runtime> Engine<R> {
    /// Runs command `c` of `p` in `tx`, over `local`, what `tx` holds outside the file. Returns
    /// its rows.
    pub(super) fn command(
        &self,
        tx: &Txn,
        p: &Prepared,
        c: &Command,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<DynRows, write::Error> {
        let schema = p.schema();
        match c {
            Command::Explain {
                analyze,
                target: Explained::Statement(t),
            } => {
                let one = self.text_of(p, std::slice::from_ref(t));
                let node = self.db.explain(tx, one, *t)?;
                let (node, extra) = if *analyze {
                    self.analyzed(tx, one, node, local, bounds)?
                } else {
                    (node, Vec::new())
                };
                let ty = Ty::Record(vec![
                    ("document".into(), Ty::Base(Base::Text)),
                    ("text".into(), Ty::Base(Base::Text)),
                ]);
                let mut shown = node.text();
                for a in &extra {
                    shown.push_str(&format!("{} {}\n", a.name, a.line));
                }
                let members: Vec<(&str, String)> =
                    extra.iter().map(|a| (a.name, a.json.clone())).collect();
                let row = Val::record(vec![Val::text(&node.document(&members)), Val::text(&shown)]);
                Ok(rows(schema, ty, vec![row]))
            }
            Command::Vacuum(rel) => self.vacuumed(schema, *rel),
            Command::Reindex {
                relation, index, ..
            } => {
                let at = reindexed(schema, &self.program, *relation, *index);
                Err(write::Error::Unsupported(format!(
                    "rebuilding indexes {at:?} of `{}`",
                    schema.relation(*relation).name
                )))
            }
            Command::SetSetting { setting, value } => {
                self.set_setting(tx, p, *setting, *value, local, bounds)
            }
            other => Err(write::Error::Unsupported(format!("`{}`", word(other)))),
        }
    }

    fn text_of<'a>(&self, p: &'a Prepared, items: &'a [Tm]) -> Text<'a> {
        Text {
            schema: p.schema(),
            terms: p.terms(),
            items,
            params: p.params(),
            clock: self.clock(),
            caller: Some(p.caller()),
        }
    }

    /// Runs the statement of `text` and undoes it. Returns `node` with what the run did, and
    /// the change set and constraint outcome, each a name, a line of text and a JSON value.
    fn analyzed(
        &self,
        tx: &Txn,
        text: Text<'_>,
        mut node: Node,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<(Node, Vec<Added>), write::Error> {
        let start = self.rt.now();
        let mut outcomes = self
            .db
            .trial(tx, text, local, bounds)
            .map_err(|f| f.error)?;
        let took = self.rt.now().saturating_since(start);
        let o: Outcome = outcomes.pop().ok_or_else(|| {
            write::Error::Internal("a trial of one statement has one outcome".into())
        })?;
        let rows = match &o.result {
            _ if *text.terms.ty(text.items[0]) == Ty::Change => 0,
            Val::Set(s) => s.len(),
            Val::List(l) => l.len(),
            _ => 1,
        };
        node.actual = Some(Actual {
            rows: rows as u64,
            loops: 1,
            time_us: u64::try_from(took.as_micros()).unwrap_or(u64::MAX),
            storage: String::new(),
        });
        let mut sizes: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let name = |r: RelId, c: u32| {
            let rel = text.schema.relation(r);
            let ctor = &text.schema.ty(rel.elem_def).ctors()[c as usize].name;
            format!("{}.{ctor}", rel.name)
        };
        for ((r, c), vs) in &o.deleted {
            sizes.entry(name(*r, *c)).or_default().0 += vs.len();
        }
        for ((r, c), vs) in &o.inserted {
            sizes.entry(name(*r, *c)).or_default().1 += vs.len();
        }
        let line: Vec<String> = sizes
            .iter()
            .map(|(n, (minus, plus))| format!("{n}: -{minus} +{plus}"))
            .collect();
        let json: Vec<(String, String)> = sizes
            .iter()
            .map(|(n, (minus, plus))| {
                (
                    n.clone(),
                    object(&[
                        ("deleted".into(), minus.to_string()),
                        ("inserted".into(), plus.to_string()),
                    ]),
                )
            })
            .collect();
        Ok((
            node,
            vec![
                Added {
                    name: "change",
                    line: line.join(", "),
                    json: object(&json),
                },
                Added {
                    name: "constraints",
                    line: "checked".into(),
                    json: json_text("checked"),
                },
            ],
        ))
    }

    /// Vacuums `rel`. Returns one row per constructor relation.
    fn vacuumed(&self, schema: &Schema, rel: RelId) -> Result<DynRows, write::Error> {
        let ctors = engine_relation(schema, "catalog", "constructors")?;
        let ty = Ty::Record(vec![
            (
                "ctor".into(),
                Ty::Key {
                    rel: ctors,
                    ctors: None,
                },
            ),
            ("freed".into(), Ty::Base(Base::Text)),
            ("removed".into(), Ty::Base(Base::I64)),
        ]);
        let out = self
            .db
            .vacuum(rel)?
            .into_iter()
            .map(|(id, report)| {
                Val::record(vec![
                    Val::Int(i64::from(id.0)),
                    Val::text(&format!(
                        "{} pages, {} out-of-line chunks",
                        report.removed, report.chunks
                    )),
                    Val::Int(i64::try_from(report.freed).unwrap_or(i64::MAX)),
                ])
            })
            .collect();
        Ok(rows(schema, ty, out))
    }

    /// Replaces the row of constructor `setting` of `catalog::settings` by one holding the
    /// fields of `value`, a term of `p`, in a transaction of its own. Returns the new row.
    fn set_setting(
        &self,
        tx: &Txn,
        p: &Prepared,
        setting: u32,
        value: Tm,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<DynRows, write::Error> {
        let schema = p.schema();
        let settings = engine_relation(schema, "catalog", "settings")?;
        let def = schema.relation(settings).elem_def;
        let given = self.text_of(p, std::slice::from_ref(&value));
        let mut got = self
            .db
            .statements(tx, given, local, bounds)
            .map_err(|f| f.error)?;
        let record = got.pop().map(|o| o.result).ok_or_else(|| {
            write::Error::Internal("a statement of one item has one outcome".into())
        })?;
        let Ty::Record(names) = p.terms().ty(value) else {
            return Err(write::Error::Internal(
                "a setting's value is a record".into(),
            ));
        };
        let given: BTreeMap<&str, &Val> = names
            .iter()
            .map(|(n, _)| n.as_str())
            .zip(record.fields()?)
            .collect();
        let ctor = &schema.ty(def).ctors()[setting as usize];
        let id = schema.ids.ctors[&(def, setting)];
        let mut fields = vec![Val::Int(i64::try_from(id).unwrap_or(i64::MAX))];
        for f in &ctor.fields {
            let v = given.get(f.name.as_str()).ok_or_else(|| {
                write::Error::Internal(format!("a setting's value has field `{}`", f.name))
            })?;
            fields.push((*v).clone());
        }
        let row = Val::ctor(setting, fields);
        self.swap_row(schema, settings, Old::Ctor(setting), row.clone())?;
        let window = {
            let mut settings = self.settings.lock(&self.rt);
            settings.set(schema, &row);
            settings.window
        };
        self.db.set_retention(window)?;
        let elem = Ty::Data {
            def,
            args: Vec::new(),
            ctors: None,
        };
        Ok(rows(schema, elem, vec![row]))
    }

    /// Replace the rows `old` names of `rel` by `row`, in a transaction of its own.
    pub(super) fn swap_row(
        &self,
        schema: &Schema,
        rel: RelId,
        old: Old,
        row: Val,
    ) -> Result<(), write::Error> {
        let elem = Ty::Data {
            def: schema.relation(rel).elem_def,
            args: Vec::new(),
            ctors: None,
        };
        let set = Ty::Set(Box::new(elem));
        let mut terms = Terms::default();
        let x = terms.var("row·", set.clone());
        let mut params = vec![(x, Val::set([row].into()))];
        let gone = match old {
            Old::Ctor(c) => terms.add(Kind::Rel(rel, Some(vec![c])), set.clone()),
            Old::Row(v) => {
                let y = terms.var("old·", set.clone());
                params.push((y, Val::set([v].into())));
                terms.add(Kind::Var(y), set.clone())
            }
        };
        let del = terms.add(Kind::Delete(rel, gone), Ty::Change);
        let new = terms.add(Kind::Var(x), set);
        let ins = terms.add(Kind::Insert(rel, new), Ty::Change);
        let both = terms.add(Kind::Par(vec![del, ins]), Ty::Change);
        let tx = self.db.begin()?;
        let text = Text {
            schema,
            terms: &terms,
            items: &[both],
            params: &params,
            clock: self.clock(),
            caller: None,
        };
        let ran = self
            .db
            .statements(&tx, text, &mut Local::default(), Bounds::default());
        match ran {
            Ok(_) => self.db.commit(tx).map(drop),
            Err(f) => {
                self.db.abort(tx)?;
                Err(f.error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use adtb_testkit::fixture::Fixture;

    use super::reindexed;

    const SCHEMA: &str = r"
type User = { id: Key users, name: Text }
relation users : User key id generate sequence
type Identity { id: Key identities, user: Key users }
  = Google { gid: Text }
  | Apple  { sub: Text }
relation identities : Identity key id generate sequence
  unique (user) in Google
  index (user) in Google
  index (user)
  unique (gid) in Google
";

    #[test]
    fn a_clause_two_declared_indexes_match_rebuilds_both() {
        let f = Fixture::new(SCHEMA);
        let r = f.rel("identities");
        assert_eq!(vec![0, 1], reindexed(&f.schema, &f.program, r, Some(0)));
        assert_eq!(vec![0, 1], reindexed(&f.schema, &f.program, r, Some(1)));
    }

    #[test]
    fn a_clause_over_another_constructor_set_or_other_fields_rebuilds_its_index_alone() {
        let f = Fixture::new(SCHEMA);
        let r = f.rel("identities");
        assert_eq!(vec![2], reindexed(&f.schema, &f.program, r, Some(2)));
        assert_eq!(vec![3], reindexed(&f.schema, &f.program, r, Some(3)));
    }

    #[test]
    fn reindex_of_a_relation_rebuilds_every_index() {
        let f = Fixture::new(SCHEMA);
        let r = f.rel("identities");
        assert_eq!(vec![0, 1, 2, 3], reindexed(&f.schema, &f.program, r, None));
    }
}
