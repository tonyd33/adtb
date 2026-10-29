use super::value::Shape;
use super::*;
use crate::kernel::{Kind, Prim};

impl Analyzer<'_> {
    pub(super) fn eval(&mut self, arena: Arena, tm: Tm, env: &Env) -> Result<Value, Error> {
        let node = self.terms(arena).node(tm).clone();
        match node.kind {
            Kind::Var(v) => env
                .get(&v)
                .cloned()
                .ok_or(Error::InvalidKernel("every free variable has provenance")),
            Kind::Const(_) | Kind::Caller | Kind::Fresh(..) | Kind::Skip => {
                Ok(self.unknown(&node.ty))
            }
            Kind::EmptySet => Ok(Value::set(Value::default())),
            Kind::Unreachable => Ok(Value::default()),
            Kind::Record(fields) => self.record(arena, None, &fields, env),
            Kind::Ctor { ctor, fields } => self.record(arena, Some(ctor), &fields, env),
            Kind::Field(row, name) => self.project(arena, tm, row, &name, env),
            Kind::Prim(prim, args) => self.opaque(arena, Some(prim), &args, &node.ty, env),
            Kind::Case { scrutinee, arms } => self.case(arena, scrutinee, &arms, env),
            Kind::If(condition, a, b) => self.branch(arena, (condition, a, b), env),
            Kind::Let(var, value, body) => {
                let value = self.eval(arena, value, env)?;
                self.bind_eval(arena, var, value, body, env)
            }
            Kind::LetBang(var, write, body) => {
                let value = self
                    .eval(arena, write, env)?
                    .binding
                    .ok_or(Error::InvalidKernel(
                        "bound write supplies its result provenance",
                    ))?;
                self.bind_eval(arena, var, *value, body, env)
            }
            Kind::Block(write, body) => {
                self.eval(arena, write, env)?;
                self.eval(arena, body, env)
            }
            Kind::Lam(..) | Kind::App(..) => Err(Error::InvalidKernel(
                "normal expression contains no function value",
            )),
            Kind::Call { args, .. } => self.opaque(arena, None, &args, &node.ty, env),
            Kind::Key(row) => {
                let value = self.eval(arena, row, env)?;
                let Ty::Key { rel, .. } = node.ty else {
                    return Err(Error::InvalidKernel("key has a key type"));
                };
                for key in self.schema.relation(rel).key.clone() {
                    self.consume(&value.field(&key));
                }
                Ok(Value::scalar())
            }
            Kind::KeyOf(_, key) => self.eval(arena, key, env),
            Kind::Deref { key, .. } => self.dereference(arena, key, env),
            Kind::Tag(row) => self.tag(arena, row, &node.ty, env),
            Kind::Try(body) => self.try_value(arena, body, &node.ty, env),
            Kind::Rel(rel, ctors) => self.relation(rel, ctors.as_deref(), None, true),
            Kind::Transition(transition, rel, ctors) => {
                self.relation(rel, ctors.as_deref(), Some(transition), true)
            }
            Kind::Restrict(source, ctors) => {
                let mut value = self.eval(arena, source, env)?;
                self.observe(&value.elements(), &ctors)?;
                value.restrict(&ctors);
                Ok(value)
            }
            Kind::Singleton(value) => Ok(Value::singleton(self.eval(arena, value, env)?)),
            Kind::Union(a, b) => {
                let mut value = self.eval(arena, a, env)?;
                let other = self.eval(arena, b, env)?;
                let empty = value.may_empty && other.may_empty;
                value.join(other);
                value.may_empty = empty;
                Ok(value)
            }
            Kind::For(var, source, body) => self.for_each(arena, var, source, body, env),
            Kind::IsEmpty(source) | Kind::Exists(source) => {
                self.eval(arena, source, env)?;
                Ok(self.unknown(&node.ty))
            }
            Kind::Agg {
                op,
                var,
                source,
                body,
            } => self.aggregate(arena, op, (var, source, body), env),
            Kind::Group { var, source, by } => self.group(arena, var, source, by, env),
            Kind::Top {
                count,
                keys,
                var,
                source,
            } => {
                let count = self.eval(arena, count, env)?;
                self.consume(&count);
                let value = self.ordered(arena, var, source, &keys, env)?;
                self.multiplicity(&value.elements());
                Ok(value)
            }
            Kind::Present {
                var,
                source,
                keys,
                take,
                after,
            } => {
                for arg in take.into_iter().chain(after) {
                    let value = self.eval(arena, arg, env)?;
                    self.consume(&value);
                }
                self.ordered(arena, var, source, &keys, env)
            }
            Kind::Fix(binds, body) => self.fix(arena, &binds, body, env),
            Kind::Only(source) => Ok(self.eval(arena, source, env)?.elements()),
            Kind::At { point, body, .. } => {
                let point = self.eval(arena, point, env)?;
                self.consume(&point);
                let snapshot = self.fresh();
                let outer = std::mem::replace(&mut self.snapshot, snapshot);
                let value = self.eval(arena, body, env);
                self.snapshot = outer;
                value
            }
            Kind::Reject(message) | Kind::Fail(message) => {
                let message = self.eval(arena, message, env)?;
                self.consume(&message);
                Ok(Value::default())
            }
            Kind::Insert(rel, rows) => self.insert(arena, rel, rows, env),
            Kind::Delete(rel, rows) => self.delete(arena, rel, rows, env),
            Kind::Replace(rel, rows, function) => self.replace(arena, rel, rows, function, env),
            Kind::Upsert {
                rel,
                rows,
                on: _,
                old,
                new,
                otherwise,
                single,
            } => self.upsert(arena, (rel, rows, old, new, otherwise, single), env),
            Kind::Rejects { into, insert } => self.rejects(arena, into, insert, env),
            Kind::Par(writes) => self.parallel(arena, &writes, env),
            Kind::Transact(body, _) => self.eval(arena, body, env),
        }
    }

    fn project(
        &mut self,
        arena: Arena,
        tm: Tm,
        row: Tm,
        name: &str,
        env: &Env,
    ) -> Result<Value, Error> {
        let value = self.eval(arena, row, env)?.field(name);
        if !self.terms(arena).transport_fields.contains(&tm) {
            self.consume(&value);
        }
        Ok(value)
    }

    fn opaque(
        &mut self,
        arena: Arena,
        prim: Option<Prim>,
        args: &[Tm],
        ty: &Ty,
        env: &Env,
    ) -> Result<Value, Error> {
        for &arg in args {
            let value = self.eval(arena, arg, env)?;
            self.consume(&value);
        }
        if matches!(prim, Some(Prim::ReadFile { .. })) {
            self.requirements.required.insert(Requirement::ReadFiles);
        }
        Ok(self.unknown(ty))
    }

    fn branch(&mut self, arena: Arena, branches: (Tm, Tm, Tm), env: &Env) -> Result<Value, Error> {
        let (condition, a, b) = branches;
        let condition = self.eval(arena, condition, env)?;
        self.consume(&condition);
        let mut out = self.eval(arena, a, env)?;
        out.join(self.eval(arena, b, env)?);
        Ok(out)
    }

    fn dereference(&mut self, arena: Arena, key: Tm, env: &Env) -> Result<Value, Error> {
        let value = self.eval(arena, key, env)?;
        self.consume(&value);
        let Ty::Key { rel, ctors } = self.terms(arena).ty(key).clone() else {
            return Err(Error::InvalidKernel("dereference has a key operand"));
        };
        Ok(self.relation(rel, ctors.as_deref(), None, true)?.elements())
    }

    fn tag(&mut self, arena: Arena, row: Tm, ty: &Ty, env: &Env) -> Result<Value, Error> {
        let value = self.eval(arena, row, env)?;
        let Ty::Data { def, ctors, .. } = self.terms(arena).ty(row).clone() else {
            return Err(Error::InvalidKernel("tag has a sum operand"));
        };
        self.observe(&value, &self.constructors(def, ctors.as_deref()))?;
        let mut out = self.unknown(ty);
        if let (Shape::Rows(source), Shape::Rows(result)) = (&value.shape, &mut out.shape) {
            result.retain(|c, _| source.contains_key(c));
        }
        Ok(out)
    }

    fn try_value(&mut self, arena: Arena, body: Tm, ty: &Ty, env: &Env) -> Result<Value, Error> {
        let value = self.eval(arena, body, env)?;
        let mut out = self.unknown(ty);
        if let Shape::Rows(rows) = &mut out.shape
            && let Some(ok) = rows.get_mut(&Some(0))
        {
            ok.insert("value".to_string(), value);
        }
        Ok(out)
    }

    fn rejects(
        &mut self,
        arena: Arena,
        into: RelId,
        insert: Tm,
        env: &Env,
    ) -> Result<Value, Error> {
        let value = self.eval(arena, insert, env)?;
        let count = self
            .schema
            .ty(self.schema.relation(into).elem_def)
            .ctors()
            .len();
        for c in 0..count as u32 {
            let target = self.target(into, c)?;
            self.requirements
                .required
                .insert(Requirement::Insert(target));
        }
        Ok(value)
    }

    fn record(
        &mut self,
        arena: Arena,
        ctor: Option<u32>,
        fields: &[(String, Tm)],
        env: &Env,
    ) -> Result<Value, Error> {
        let mut values = BTreeMap::new();
        for (name, tm) in fields {
            values.insert(name.clone(), self.eval(arena, *tm, env)?);
        }
        Ok(Value::row(ctor, values))
    }

    fn case(
        &mut self,
        arena: Arena,
        scrutinee: Tm,
        arms: &[(u32, Var, Tm)],
        env: &Env,
    ) -> Result<Value, Error> {
        let value = self.eval(arena, scrutinee, env)?;
        let ctors: Vec<u32> = arms.iter().map(|&(c, _, _)| c).collect();
        self.observe(&value, &ctors)?;
        let mut out = Value::default();
        for row in value.alternatives() {
            let Shape::Rows(rows) = &row.shape else {
                return Err(Error::InvalidKernel("case scrutinizes a sum"));
            };
            for (c, v, b) in arms {
                if rows.contains_key(&Some(*c)) {
                    out.join(self.bind_eval(arena, *v, row.clone(), *b, env)?);
                }
            }
        }
        Ok(out)
    }

    /// Charge the constructors `ctors` of every relation `value` was read from, as ranging them does.
    fn observe(&mut self, value: &Value, ctors: &[u32]) -> Result<(), Error> {
        for &rel in &value.stored {
            for &c in ctors {
                let target = self.target(rel, c)?;
                self.range(target);
            }
        }
        Ok(())
    }

    /// Returns the union of `body` over `source`, unkeying rows of the snapshots `body` opens.
    fn for_each(
        &mut self,
        arena: Arena,
        var: Var,
        source: Tm,
        body: Tm,
        env: &Env,
    ) -> Result<Value, Error> {
        let value = self.eval(arena, source, env)?.elements();
        let floor = self.next_id;
        let mut out = Value::set(Value::default());
        for element in value.alternatives() {
            out.join(self.bind_eval(arena, var, element, body, env)?);
        }
        out.unkey_after(floor);
        Ok(out)
    }
}
