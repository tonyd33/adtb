//! The commands of ad hoc text run in process, from their checked terms.

use std::collections::BTreeMap;

use adtb_lang::ast::{self, BundleName, ExprKind, KeyArgs, RecordExpr};
use adtb_lang::check::{Command, Explained, Program};
use adtb_lang::ids::{CatConstraintId, held_id};
use adtb_lang::kernel::{Kind, Terms, Tm, alpha_equal};
use adtb_lang::resolve::Visible;
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::textual::{Form, print};
use adtb_lang::types::{Base, CtorSet, RelId, Ty};

use crate::exec::Val;
use crate::plan::explain::{Actual, Node, json_text, object};
use crate::runtime::Runtime;
use crate::write::{self, Bounds, HoldWait, Local, Outcome, Rejection, Text, Txn, VacuumHold};

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
        | Command::SupersedeBundle(..)
        | Command::CancelCall(_)
        | Command::PrintSchema
        | Command::Export { .. }
        | Command::Restore { .. }
        | Command::Dump { .. }
        | Command::Backup {
            incremental: false, ..
        } => Ok(()),
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
        C::Vacuum(_) | C::Verify(_) | C::Compact | C::FinalizeUpgrade => {
            Need::Capabilities(&[Maintain])
        }
        C::Backup { .. } | C::Dump { .. } => Need::Capabilities(&[Maintain, WriteFiles]),
        C::TerminateSession(k) if key_value(k, params) => Need::Capabilities(&[Cancel]),
        C::SetSetting(_, v) if value(v, params) => Need::Capabilities(&[Configure]),
        C::RotateKek(_) | C::RotateDek | C::DestroyDek(_) => Need::Capabilities(&[Configure]),
        C::ApplyMigrations(_) => Need::Capabilities(&[Schema]),
        C::ArchiveAudit { through, .. } if value(through, params) => {
            Need::Capabilities(&[Maintain, WriteFiles])
        }
        C::RetireBundle(b) | C::SupersedeBundle(b, _) => Need::BundleOwner(b),
        C::Explain {
            target: ast::ExplainTarget::Registered { .. },
            ..
        } => Need::SchemaOwner,
        C::Explain { .. } | C::PrintSchema => Need::Capabilities(&[]),
        C::CancelCall(k) if key_value(k, params) => Need::Capabilities(&[]),
        C::Export { .. } => Need::Capabilities(&[WriteFiles]),
        C::PinPlan(_)
        | C::UnpinPlan(_)
        | C::Reindex { .. }
        | C::TerminateSession(_)
        | C::SetSetting(..)
        | C::ArchiveAudit { .. }
        | C::CancelCall(_)
        | C::RegisterBundle(_)
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
        Command::Explain { .. }
            | Command::Verify(_)
            | Command::Vacuum(_)
            | Command::Reindex { .. }
            | Command::CancelCall(_)
            | Command::PrintSchema
            | Command::Export { .. }
            | Command::Backup { .. }
            | Command::Dump { .. }
            | Command::Restore { .. }
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

/// Returns the CSV of `rows` of type `elem`, a header of column names then a line per row.
fn export_csv<'v>(
    schema: &Schema,
    elem: &Ty,
    rows: impl Iterator<Item = &'v Val>,
    sums_text: bool,
) -> Result<Vec<u8>, write::Error> {
    let columns = columns(schema, elem).ok_or_else(|| {
        write::Error::Exec(crate::exec::Error::Unsupported(
            "`export` of rows that are not flat records".into(),
        ))
    })?;
    for (name, ty) in &columns {
        let plain = match ty {
            Ty::Base(_) | Ty::Key { .. } => true,
            Ty::Data { .. } => sums_text,
            _ => false,
        };
        if !plain {
            return Err(write::Error::Exec(crate::exec::Error::Unsupported(
                format!("`export` of column `{name}` to CSV"),
            )));
        }
    }
    let line = |cells: Vec<String>| {
        let quoted: Vec<String> = cells.iter().map(|c| csv_cell(c)).collect();
        format!("{}\n", quoted.join(","))
    };
    let mut out = line(columns.iter().map(|(n, _)| n.clone()).collect());
    for row in rows {
        let fields = match row {
            Val::Ctor(_, fields) => fields.as_ref(),
            other => other.fields()?,
        };
        let mut cells = Vec::with_capacity(columns.len());
        for ((_, ty), value) in columns.iter().zip(fields) {
            cells.push(match value {
                Val::Text(t) => t.to_string(),
                other => print(other, &Form::of(schema, ty)).map_err(write::Broken::Unprintable)?,
            });
        }
        out.push_str(&line(cells));
    }
    Ok(out.into_bytes())
}

/// Returns the columns of a row of type `ty`: a record's fields, or those of a sum of one
/// constructor.
fn columns(schema: &Schema, ty: &Ty) -> Option<Vec<(String, Ty)>> {
    match ty {
        Ty::Record(fields) => Some(fields.clone()),
        Ty::Data { def, args, .. } => match schema.ty(*def).ctors() {
            [only] => Some(
                schema
                    .ty(*def)
                    .header()
                    .iter()
                    .chain(&only.fields)
                    .map(|f| (f.name.clone(), f.ty.subst(args)))
                    .collect(),
            ),
            _ => None,
        },
        _ => None,
    }
}

fn csv_cell(cell: &str) -> String {
    if cell.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_owned()
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
        Command::Vacuum(_)
        | Command::Verify(_)
        | Command::Reindex { .. }
        | Command::CancelCall(_)
        | Command::PrintSchema
        | Command::Export { .. }
        | Command::Backup { .. }
        | Command::Restore { .. } => Err(Error::invalid(format!(
            "`{}` does not run inside a transaction",
            word(c)
        ))),
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
) -> Result<RelId, write::Broken> {
    schema
        .module_named(module)
        .and_then(|m| match schema.module(m).values.get(name) {
            Some(ValueDef::Relation(r)) => Some(*r),
            _ => None,
        })
        .ok_or_else(|| write::Broken::Undeclared {
            module: module.to_string(),
            name: name.to_string(),
        })
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
                let (mut node, extra) = if *analyze {
                    self.analyzed(tx, one, node, local, bounds)?
                } else {
                    (node, Vec::new())
                };
                if let Some(holds) = p.issuer.as_ref().and_then(|i| i.holds.as_ref()) {
                    node.hide(holds.monitor, holds.schema);
                }
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
                let held = self.db.hold_vacuum(HoldWait::of(&bounds, at))?;
                self.set_setting(tx, p, (*setting, *value), local, bounds, &held)
            }
            Command::CancelCall(key) => self.cancel_call(tx, p, *key, local, bounds),
            Command::PrintSchema => {
                let ty = Ty::Record(vec![("text".into(), Ty::Base(Base::Text))]);
                let row = Val::record(vec![Val::text(&self.printed_schema())]);
                Ok(rows(schema, ty, vec![row]))
            }
            Command::Export {
                source,
                path,
                sums_text,
            } => self.export(tx, p, (*source, path, *sums_text), local, bounds),
            other => Err(write::Error::Exec(crate::exec::Error::Unsupported(
                format!("`{}`", word(other)),
            ))),
        }
    }

    /// Returns the folded schema: its version and hash, then each module's declarations.
    fn printed_schema(&self) -> String {
        let hash = crate::catalog::version_hash(&self.schema, &self.source);
        let hash: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        let mut out = format!("-- version {} hash {hash}\n", self.source.version);
        let mut module = None;
        for d in &self.source.decls {
            if module != Some(&d.module) {
                out.push_str(&format!("module {}\n", d.module));
                module = Some(&d.module);
            }
            out.push_str(&d.text);
            out.push('\n');
        }
        out
    }

    /// Returns the value of `t`, a term of `p` reading no relation.
    pub(super) fn value_of(
        &self,
        tx: &Txn,
        p: &Prepared,
        t: Tm,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<Val, write::Error> {
        let given = self.text_of(p, std::slice::from_ref(&t));
        let mut got = self
            .db
            .statements(tx, given, local, bounds)
            .map_err(|f| f.error)?;
        got.pop()
            .map(|o| o.result)
            .ok_or_else(|| write::Broken::NoValueOutcome.into())
    }

    /// Record that the issuer of `p` was refused while its text ran. Returns the refusal.
    pub(super) fn refused_running(
        &self,
        p: &Prepared,
        issuer: &Issuer,
        detail: String,
    ) -> write::Error {
        match self.audit_denied(&issuer.role, p.caller(), &CallOrigin::AdHoc(issuer.text)) {
            Ok(()) => write::Error::Denied(detail),
            Err(e) => write::Broken::RefusalUnrecorded(Box::new(e)).into(),
        }
    }

    /// Stop the call of key `key`. A role without `cancel` stops only calls of its own session, and
    /// its refusal does not say whether the call exists. Returns the row of the call stopped.
    fn cancel_call(
        &self,
        tx: &Txn,
        p: &Prepared,
        key: Tm,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<DynRows, write::Error> {
        let schema = p.schema();
        let Val::Int(id) = self.value_of(tx, p, key, local, bounds)? else {
            return Err(write::Broken::CallKeyNotInt.into());
        };
        let calls = self.calls.lock(&self.rt);
        let session = calls.session(id);
        if let Some(issuer) = &p.issuer
            && !issuer.holds.as_ref().is_some_and(|h| h.cancel)
            && session != Some(issuer.session)
        {
            drop(calls);
            let detail = format!("role `{}` lacks `cancel` for this call", issuer.role);
            return Err(self.refused_running(p, issuer, detail));
        }
        calls.stop(id);
        drop(calls);
        let ty = Ty::Record(vec![(
            "call".into(),
            Ty::Key {
                rel: engine_relation(schema, "sys", "calls")?,
                ctors: None,
            },
        )]);
        let stopped = session.map(|_| Val::record(vec![Val::Int(id)]));
        Ok(rows(schema, ty, stopped.into_iter().collect()))
    }

    /// Write the rows of `source` to `path` by its extension.
    /// Returns the path and the number of rows.
    fn export(
        &self,
        tx: &Txn,
        p: &Prepared,
        (source, path, sums_text): (Tm, &str, bool),
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<DynRows, write::Error> {
        let schema = p.schema();
        let confined = bounds.confined;
        if !path.ends_with(".csv") {
            return Err(write::Error::Exec(crate::exec::Error::Unsupported(
                format!("`export` to `{path}`; only CSV is written"),
            )));
        }
        let Ty::Set(elem) = p.terms().ty(source) else {
            return Err(write::Broken::ExportNotSetTyped.into());
        };
        let Val::Set(found) = self.value_of(tx, p, source, local, bounds)? else {
            return Err(write::Broken::ExportNotSet.into());
        };
        let bytes = export_csv(schema, elem, found.iter(), sums_text)?;
        let outside = format!("path `{path}` is outside export_roots");
        self.db
            .files()
            .write(path, &bytes, confined)
            .map_err(|refused| match (refused, &p.issuer) {
                (adtb_file::Refused::Outside(_), Some(issuer)) => {
                    self.refused_running(p, issuer, outside.clone())
                }
                (adtb_file::Refused::Outside(_), None) => write::Error::Denied(outside.clone()),
                (adtb_file::Refused::Unreadable(detail), _) => {
                    write::Error::Rejected(Rejection::BadInput {
                        source: path.to_owned(),
                        line: 0,
                        detail,
                    })
                }
            })?;
        let ty = Ty::Record(vec![
            ("path".into(), Ty::Base(Base::Text)),
            ("rows".into(), Ty::Base(Base::I64)),
        ]);
        let count = i64::try_from(found.len()).unwrap_or(i64::MAX);
        let row = Val::record(vec![Val::text(path), Val::Int(count)]);
        Ok(rows(schema, ty, vec![row]))
    }

    pub(super) fn text_of<'a>(&self, p: &'a Prepared, items: &'a [Tm]) -> Text<'a> {
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
        let o: Outcome = outcomes.pop().ok_or(write::Broken::NoTrialOutcome)?;
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
        let violation = schema.engine_ctors().violation().index();
        let finding = schema.ty(schema.relation(findings).elem_def);
        let constraint_at = finding.header().len();
        let holds = issuer.holds.as_ref();
        let unreadable = |row: &Val| {
            let Val::Ctor(c, fields) = row else {
                return false;
            };
            if *c != violation {
                return false;
            }
            let at = match fields.get(constraint_at) {
                Some(Val::Int(i)) => u64::try_from(*i)
                    .ok()
                    .and_then(|id| schema.ids().constraint_of(CatConstraintId::from_raw(id))),
                _ => None,
            };
            !at.zip(holds)
                .is_some_and(|(at, h)| h.reads_constraint(self, at))
        };
        if !found.any_left(unreadable) {
            return Ok(());
        }
        self.audit_denied(&issuer.role, caller, &CallOrigin::AdHoc(issuer.text))
            .map_err(|e| write::Broken::WitnessDenialUnrecorded(Box::new(e)))?;
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
        let done = self.db.vacuum(rel, waiting)?;
        let chunks = done.report.chunks;
        let out = (done.ctors.into_iter())
            .map(|(id, heap)| {
                Val::record(vec![
                    Val::Int(i64::from(id.0)),
                    Val::text(&format!(
                        "{} pages, {chunks} out-of-line chunks of the relation",
                        heap.removed
                    )),
                    Val::Int(i64::try_from(heap.freed).unwrap_or(i64::MAX)),
                ])
            })
            .collect();
        Ok(rows(schema, ty, out))
    }

    /// Replaces the row of constructor `setting` of `catalog::settings` by one holding the
    /// fields of `value`, a term of `p`, in a transaction of its own. Returns the new row.
    ///
    /// A lease setting first latches every bundle expired under the old values.
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
        let record = got
            .pop()
            .map(|o| o.result)
            .ok_or(write::Broken::NoSettingOutcome)?;
        let Ty::Record(names) = p.terms().ty(value) else {
            return Err(write::Broken::SettingNotRecord.into());
        };
        let given: BTreeMap<&str, &Val> = names
            .iter()
            .map(|(n, _)| n.as_str())
            .zip(record.fields()?)
            .collect();
        let ctor = &schema.ty(def).ctors()[setting as usize];
        let id = schema.ty(def).ctor_at(setting);
        let id = held_id(id.and_then(|c| schema.ids().ctor_id(c))).raw();
        let mut fields = vec![Val::Int(i64::try_from(id).unwrap_or(i64::MAX))];
        for f in &ctor.fields {
            let v = given
                .get(f.name.as_str())
                .ok_or_else(|| write::Broken::SettingField(f.name.clone()))?;
            fields.push((*v).clone());
        }
        let row = Val::ctor(setting, fields);
        let writer = self.deployed.write(&self.rt);
        if matches!(ctor.name.as_str(), "DefaultLease" | "LeaseGrace") {
            self.latch_expired(&writer)?;
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
            Err(f) => Err(self.db.abandoned(tx, f.error)),
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
