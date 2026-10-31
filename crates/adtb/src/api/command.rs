//! The commands of ad hoc text run in process, from their checked terms.

use std::collections::BTreeMap;

use adtb_lang::ast::{self, BundleName, ExprKind, KeyArgs, RecordExpr};
use adtb_lang::check::{Command, Explained, Program};
use adtb_lang::kernel::{Kind, Terms, Tm, alpha_equal};
use adtb_lang::resolve::Visible;
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::{Base, CtorSet, RelId, Ty};

use crate::exec::Val;
use crate::plan::explain::{Actual, Node, json_text, object};
use crate::runtime::Runtime;
use crate::write::{self, Bounds, HoldWait, Local, Outcome, Text, Txn, VacuumHold};

use super::server::CallOrigin;
use super::text::{DynRows, Issuer, Prepared, rows};
use super::{Capability, Engine, Error, unsupported};

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
        | Command::Verify(_)
        | Command::Reindex { .. }
        | Command::SetSetting { .. }
        | Command::RetireBundle(_)
        | Command::SupersedeBundle(..) => Ok(()),
        other => Err(unsupported(&format!("`{}`", word(other)))),
    }
}

/// What a role but the schema owner must hold to issue a parsed command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Need<'c> {
    Capabilities(&'static [Capability]),
    BundleOwner(&'c BundleName),
    /// More than capabilities decide: reads the role must hold, the calls it owns, or a value that
    /// may read a relation. No role issues it.
    SchemaOwner,
}

/// Returns what a role must hold to issue `c`, a free name in `params` standing for a value.
pub(super) fn need<'c>(c: &'c ast::Command, params: &[String]) -> Need<'c> {
    use Capability::{Cancel, Configure, Maintain, Schema, WriteFiles};
    use ast::Command as C;
    match c {
        C::PinPlan(k) | C::UnpinPlan(k) if key_value(k, params) => Need::Capabilities(&[Maintain]),
        C::Reindex { index, .. }
            if index.as_ref().is_none_or(|(fields, _)| {
                fields.iter().all(|f| field_path(f) || value(f, params))
            }) =>
        {
            Need::Capabilities(&[Maintain])
        }
        C::Vacuum(_) | C::Verify(_) | C::Compact | C::FinalizeUpgrade | C::Backup { .. } => {
            Need::Capabilities(&[Maintain])
        }
        C::TerminateSession(k) if key_value(k, params) => Need::Capabilities(&[Cancel]),
        C::SetSetting(_, v) if value(v, params) => Need::Capabilities(&[Configure]),
        C::RotateKek(_) | C::RotateDek | C::DestroyDek(_) => Need::Capabilities(&[Configure]),
        C::ApplyMigrations(_) => Need::Capabilities(&[Schema]),
        C::ArchiveAudit { through, .. } if value(through, params) => {
            Need::Capabilities(&[Maintain, WriteFiles])
        }
        C::RetireBundle(b) | C::SupersedeBundle(b, _) => Need::BundleOwner(b),
        C::PinPlan(_)
        | C::UnpinPlan(_)
        | C::Reindex { .. }
        | C::TerminateSession(_)
        | C::SetSetting(..)
        | C::ArchiveAudit { .. }
        | C::Explain { .. }
        | C::CancelCall(_)
        | C::Export { .. }
        | C::RegisterBundle(_)
        | C::PrintSchema
        | C::Dump { .. }
        | C::Restore { .. } => Need::SchemaOwner,
    }
}

/// Returns those of `params` that neither `caller` nor a name the schema declares shadows.
pub(super) fn free_params(schema: &Schema, params: &[String]) -> Vec<String> {
    let visible = Visible::all(schema);
    let declared = |name: &str| {
        name == "caller"
            || visible
                .modules
                .iter()
                .any(|&m| schema.module(m).values.contains_key(name))
    };
    params.iter().filter(|n| !declared(n)).cloned().collect()
}

/// Returns whether `c` changes data when it runs, outside the call's transaction.
pub(super) fn changes_data(c: &Command) -> bool {
    !matches!(
        c,
        Command::Explain { .. } | Command::Verify(_) | Command::Vacuum(_) | Command::Reindex { .. }
    )
}

/// Returns whether `e` computes from literals and `params` alone, reading no relation. A field
/// selection counts as reading.
fn value(e: &ast::Expr, params: &[String]) -> bool {
    let of = |e: &ast::Expr| value(e, params);
    let record = |r: &RecordExpr| matches!(r, RecordExpr::Fields(fields, _) if fields.iter().all(|(_, v)| of(v)));
    match &e.kind {
        ExprKind::Lit(_) | ExprKind::Unit => true,
        ExprKind::Name(q) => q.module.is_none() && params.contains(&q.name.text),
        ExprKind::Unary(_, inner) => of(inner),
        ExprKind::Binary(_, a, b) => of(a) && of(b),
        ExprKind::Record(r) => record(r),
        ExprKind::Ctor { record: r, .. } => r.as_ref().is_none_or(record),
        _ => false,
    }
}

/// Returns whether `e` is a [`value`] or a relation's `key` of values.
fn key_value(e: &ast::Expr, params: &[String]) -> bool {
    match &e.kind {
        ExprKind::KeyCall(relation, args) => {
            matches!(relation.kind, ExprKind::Name(_))
                && match args {
                    KeyArgs::Value(v) => value(v, params),
                    KeyArgs::Named(fields) => fields.iter().all(|(_, v)| value(v, params)),
                }
        }
        _ => value(e, params),
    }
}

/// Returns whether `e` is a name followed by field selections.
fn field_path(e: &ast::Expr) -> bool {
    match &e.kind {
        ExprKind::Name(_) => true,
        ExprKind::Field(inner, _) => field_path(inner),
        _ => false,
    }
}

/// Returns whether `c` waits for the vacuum lock when it runs.
pub(super) fn holds_off_vacuum(c: &Command) -> bool {
    matches!(
        c,
        Command::Verify(_) | Command::Vacuum(_) | Command::SetSetting { .. }
    )
}

/// Refuse a command that belongs to no transaction, for running inside one.
pub(super) fn outside_transaction(c: &Command) -> Result<(), Error> {
    match c {
        Command::Vacuum(_) | Command::Verify(_) | Command::Reindex { .. } => Err(Error::invalid(
            format!("`{}` does not run inside a transaction", word(c)),
        )),
        _ => Ok(()),
    }
}

/// Returns the words a command starts with.
pub(super) fn word(c: &Command) -> &'static str {
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
pub(super) fn engine_relation(
    schema: &Schema,
    module: &str,
    name: &str,
) -> Result<RelId, write::Error> {
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
        (c, at): (&Command, usize),
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
            Command::Vacuum(rel) => self.vacuumed(schema, *rel, HoldWait::of(&bounds, at)),
            Command::Verify(rel) => {
                let found = self.verified(schema, *rel, HoldWait::of(&bounds, at))?;
                if let Some(issuer) = &p.issuer {
                    self.refuse_witnesses(schema, issuer, p.caller(), &found)?;
                }
                Ok(found)
            }
            Command::Reindex {
                relation,
                index,
                online,
            } => {
                let at = index.map(|_| reindexed(schema, &self.program, *relation, *index));
                self.reindex_job(schema, *relation, at.as_deref(), *online)
            }
            Command::SetSetting { setting, value } => {
                let _writer = self.bundle_writer.lock(&self.rt);
                let held = self.db.hold_vacuum(HoldWait::of(&bounds, at))?;
                self.set_setting(tx, p, (*setting, *value), local, bounds, &held)
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

    /// Refuse the rows `found` of `verify` for `issuer` when a `Violation` among them holds a
    /// witness, which reveals field values, recording the refusal.
    fn refuse_witnesses(
        &self,
        schema: &Schema,
        issuer: &Issuer,
        caller: &Val,
        found: &DynRows,
    ) -> Result<(), write::Error> {
        let findings = engine_relation(schema, "sys", "verify_findings")?;
        let violation = schema
            .ty(schema.relation(findings).elem_def)
            .ctor_index("Violation")
            .ok_or_else(|| write::Error::Internal("`Finding` declares `Violation`".into()))?;
        if !found.any_left(|row| matches!(row, Val::Ctor(c, _) if *c == violation)) {
            return Ok(());
        }
        self.audit_denied(&issuer.role, caller, &CallOrigin::AdHoc(issuer.text))
            .map_err(|e| write::Error::Internal(format!("recording a denied call: {e:?}")))?;
        Err(write::Error::Denied(format!(
            "role `{}` lacks `read` on the rows of the `Violation` witnesses of `verify`",
            issuer.role
        )))
    }

    /// Vacuums `rel`. Returns one row per constructor relation.
    fn vacuumed(
        &self,
        schema: &Schema,
        rel: RelId,
        waiting: HoldWait<'_>,
    ) -> Result<DynRows, write::Error> {
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
            .vacuum(rel, waiting)?
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
    ///
    /// A lease setting first latches every bundle expired under the old values.
    ///
    /// Precondition: the caller holds `bundle_writer`.
    fn set_setting(
        &self,
        tx: &Txn,
        p: &Prepared,
        (setting, value): (u32, Tm),
        local: &mut Local,
        bounds: Bounds,
        held: &VacuumHold<'_, R>,
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
        if matches!(ctor.name.as_str(), "DefaultLease" | "LeaseGrace") {
            self.latch_expired()?;
        }
        self.swap_row(schema, settings, Old::Ctor(setting), row.clone())?;
        let window = {
            let mut settings = self.settings.lock(&self.rt);
            settings.set(schema, &row);
            self.background.configure(&settings);
            settings.window
        };
        self.db.set_retention(window, held)?;
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
