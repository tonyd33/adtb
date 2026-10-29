use super::*;
use crate::kernel::Agg;

impl Analyzer<'_> {
    pub(super) fn fix(
        &mut self,
        arena: Arena,
        binds: &[(Var, Tm)],
        body: Tm,
        env: &Env,
    ) -> Result<Value, Error> {
        let mut inner = env.clone();
        for (var, _) in binds {
            inner.insert(*var, Value::set(Value::default()));
        }
        loop {
            let previous = inner.clone();
            for (var, tm) in binds {
                let value = self.eval(arena, *tm, &previous)?;
                inner
                    .get_mut(var)
                    .expect("each fix binder is initialized")
                    .join(value);
            }
            if inner == previous {
                break;
            }
        }
        self.eval(arena, body, &inner)
    }

    pub(super) fn ordered(
        &mut self,
        arena: Arena,
        var: Var,
        source: Tm,
        keys: &[(Tm, bool)],
        env: &Env,
    ) -> Result<Value, Error> {
        let value = self.eval(arena, source, env)?;
        for row in value.elements().alternatives() {
            for (key, _) in keys {
                let key = self.bind_eval(arena, var, row.clone(), *key, env)?;
                self.consume(&key);
            }
        }
        Ok(value)
    }

    pub(super) fn group(
        &mut self,
        arena: Arena,
        var: Var,
        source: Tm,
        by: Tm,
        env: &Env,
    ) -> Result<Value, Error> {
        let value = self.eval(arena, source, env)?.elements();
        let mut out = Value::set(Value::default());
        for row in value.alternatives() {
            let key = self.bind_eval(arena, var, row.clone(), by, env)?;
            self.consume(&key);
            out.join(Value::set(Value::row(
                None,
                BTreeMap::from([
                    ("key".to_string(), key),
                    ("rows".to_string(), Value::set(row)),
                ]),
            )));
        }
        Ok(out)
    }

    pub(super) fn aggregate(
        &mut self,
        arena: Arena,
        op: Agg,
        binding: (Var, Tm, Tm),
        env: &Env,
    ) -> Result<Value, Error> {
        let (var, source, body) = binding;
        let nonempty = matches!(self.terms(arena).ty(source), Ty::NonEmpty(_));
        let source = self.eval(arena, source, env)?.elements();
        let mut input = Value::default();
        for row in source.alternatives() {
            input.join(self.bind_eval(arena, var, row, body, env)?);
        }
        if !matches!(op, Agg::Min | Agg::Max) {
            self.multiplicity(&source);
        }
        let value = match op {
            Agg::Count => Value::scalar(),
            Agg::Sum | Agg::Avg => {
                self.consume(&input);
                Value::scalar()
            }
            Agg::Min | Agg::Max => {
                self.consume(&input);
                input
            }
            Agg::User(id) => self.user_aggregate(id, input)?,
        };
        if !nonempty && matches!(op, Agg::Min | Agg::Max | Agg::Avg | Agg::User(_)) {
            let mut out = Value::row(Some(0), BTreeMap::new());
            out.join(Value::row(
                Some(1),
                BTreeMap::from([("value".to_string(), value)]),
            ));
            Ok(out)
        } else {
            Ok(value)
        }
    }

    /// Consume what tells `elements` apart from each other, skipping keyed rows.
    pub(super) fn multiplicity(&mut self, elements: &Value) {
        let mut reads = Vec::new();
        elements.each_distinguishing_read(&mut |r| reads.push(r.clone()));
        self.charge(reads);
    }

    fn user_aggregate(&mut self, id: u32, input: Value) -> Result<Value, Error> {
        let parts = self
            .aggregates
            .get(id as usize)
            .and_then(Option::as_ref)
            .ok_or(Error::InvalidKernel(
                "a reachable aggregate has normalized parts",
            ))?;
        let init = parts.init;
        let (step_vars, step) = parts.step.clone();
        let (merge_vars, merge) = parts.merge.clone();
        let (finish_vars, finish) = parts.finish.clone();
        let empty = Env::new();
        let mut state = self.eval(Arena::Program, init, &empty)?;
        loop {
            let previous = state.clone();
            let env =
                BTreeMap::from([(step_vars[0], state.clone()), (step_vars[1], input.clone())]);
            state.join(self.eval(Arena::Program, step, &env)?);
            let env = BTreeMap::from([
                (merge_vars[0], state.clone()),
                (merge_vars[1], state.clone()),
            ]);
            state.join(self.eval(Arena::Program, merge, &env)?);
            if state == previous {
                break;
            }
        }
        self.bind_eval(Arena::Program, finish_vars[0], state, finish, &empty)
    }
}
