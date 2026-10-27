//! The judgement that a value of the reference inhabits a type of the checker.

use std::fmt;

use adtb_lang::check::Program;
use adtb_lang::schema::{Schema, TypeDefKind};
use adtb_lang::textual::{Textual, key_field_ty};
use adtb_lang::types::{Base, CtorSet, RelId, Ty};
use adtb_reference::{Database, Interpreter, Value};
use unicode_normalization::is_nfc;

/// A part of a value that does not inhabit the type at its position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    /// The fields, constructors and elements from the whole value down to the part, as `.f`,
    /// `#C` and `[i]`; empty at the whole value.
    pub path: String,
    pub value: String,
    pub ty: String,
    pub why: String,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = if self.path.is_empty() {
            String::new()
        } else {
            format!("at `{}`: ", self.path)
        };
        write!(f, "{at}{} is not a `{}`: {}", self.value, self.ty, self.why)
    }
}

/// Returns whether `value` inhabits `ty`, a type of `schema`, by its structure alone.
pub fn inhabits(value: &Value, ty: &Ty, schema: &Schema) -> Result<(), Mismatch> {
    Judge::new(schema).value(value, ty)
}

/// The database a judgement reads, with the program that holds its schema's domains.
#[derive(Clone, Copy)]
struct State<'a> {
    program: &'a Program,
    db: &'a Database,
}

/// Judges values of the types of one schema.
#[derive(Clone, Copy)]
pub struct Judge<'a> {
    schema: &'a Schema,
    state: Option<State<'a>>,
    stored: bool,
}

impl fmt::Debug for Judge<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Judge")
            .field("over_a_database", &self.state.is_some())
            .field("stored", &self.stored)
            .finish()
    }
}

impl<'a> Judge<'a> {
    /// Returns the judgement by structure alone.
    pub fn new(schema: &'a Schema) -> Judge<'a> {
        Judge {
            schema,
            state: None,
            stored: false,
        }
    }

    /// Returns the judgement over `db`, where a key of a constructor set narrower than its
    /// relation's names a row of that set in `db`.
    pub fn over(schema: &'a Schema, program: &'a Program, db: &'a Database) -> Judge<'a> {
        Judge {
            schema,
            state: Some(State { program, db }),
            stored: false,
        }
    }

    /// Returns whether `value` inhabits `ty`.
    pub fn value(&self, value: &Value, ty: &Ty) -> Result<(), Mismatch> {
        self.at(&mut String::new(), value, ty)
    }

    /// Returns whether `value`, the final result of a statement of type `ty`, inhabits it. A
    /// result ordered by `order by` is a list of the elements of its set type, each once.
    pub fn result(&self, value: &Value, ty: &Ty) -> Result<(), Mismatch> {
        match (value, ty) {
            (Value::List(xs), Ty::Set(elem) | Ty::NonEmpty(elem)) => {
                let path = &mut String::new();
                if matches!(ty, Ty::NonEmpty(_)) && xs.is_empty() {
                    return Err(self.mismatch(path, value, ty, "it is empty"));
                }
                for (i, x) in xs.iter().enumerate() {
                    if xs[..i].contains(x) {
                        return Err(self.mismatch(path, value, ty, "an element repeats"));
                    }
                    self.elem(path, i, x, elem)?;
                }
                Ok(())
            }
            _ => self.value(value, ty),
        }
    }

    /// Returns whether `row` is a row of constructor `ctor` of `rel` as the database holds it:
    /// of the row type of `ctor`, every key in it naming a row of its constructor set in the
    /// database, and every value of a newtype in it within the newtype's domain.
    ///
    /// Panics when the judgement reads no database.
    pub fn row(&self, rel: RelId, ctor: u32, row: &Value) -> Result<(), Mismatch> {
        assert!(
            self.state.is_some(),
            "a row is judged over the database that holds it"
        );
        let r = self.schema.relation(rel);
        let ty = Ty::Data {
            def: r.elem_def,
            args: r.elem_args().to_vec(),
            ctors: Some(vec![ctor]),
        };
        Judge {
            stored: true,
            ..*self
        }
        .value(row, &ty)
    }

    fn mismatch(&self, path: &str, value: &Value, ty: &Ty, why: impl Into<String>) -> Mismatch {
        Mismatch {
            path: path.to_string(),
            value: format!("{value:?}"),
            ty: self.schema.show_ty(ty),
            why: why.into(),
        }
    }

    fn at(&self, path: &mut String, value: &Value, ty: &Ty) -> Result<(), Mismatch> {
        let fail = |why: &str| Err(self.mismatch(path, value, ty, why));
        match (ty, value) {
            (Ty::Base(b), _) => match base(*b, value) {
                Ok(()) => Ok(()),
                Err(why) => fail(why),
            },
            (Ty::Data { def, args, ctors }, _) => match &self.schema.ty(*def).kind {
                TypeDefKind::Newtype { base, .. } => {
                    self.at(path, value, &base.subst(args))?;
                    self.domain(path, value, ty)
                }
                TypeDefKind::Sum { .. } => self.data(path, value, ty, ctors.as_ref()),
            },
            (Ty::Sum { ctors, sel }, Value::Ctor(c, fs)) => {
                let Some((name, want)) = ctors.get(*c as usize) else {
                    return fail("its constructor is not one of the sum's");
                };
                if sel.as_ref().is_some_and(|s| !s.contains(c)) {
                    return fail("its constructor is outside the type's constructor set");
                }
                self.fields(path, &format!("#{name}"), value, ty, fs, want)
            }
            (Ty::Record(want), Value::Record(fs)) => self.fields(path, "", value, ty, fs, want),
            (Ty::Unit, Value::Record(fs)) if fs.is_empty() => Ok(()),
            (Ty::Tag(inner), Value::Ctor(c, fs)) => {
                let count = match inner.as_ref() {
                    Ty::Data { def, .. } => self.schema.ty(*def).ctors().len(),
                    Ty::Sum { ctors, .. } => ctors.len(),
                    _ => return fail("a tag is of a sum"),
                };
                match (fs.is_empty(), (*c as usize) < count) {
                    (true, true) => Ok(()),
                    (false, _) => fail("a tag has no fields"),
                    (true, false) => fail("its constructor is not one of the sum's"),
                }
            }
            (Ty::Key { rel, ctors }, Value::Key(k)) => self.key(path, value, ty, *rel, ctors, k),
            (Ty::Set(elem) | Ty::NonEmpty(elem), Value::Set(xs)) => {
                if matches!(ty, Ty::NonEmpty(_)) && xs.is_empty() {
                    return fail("it is empty");
                }
                for (i, x) in xs.iter().enumerate() {
                    self.elem(path, i, x, elem)?;
                }
                Ok(())
            }
            (Ty::Group(key, elem), Value::Record(fs)) => match fs.as_slice() {
                [(k, _), (r, Value::Set(rows))] if k == "key" && r == "rows" && rows.is_empty() => {
                    fail("a group's rows are empty")
                }
                [(k, kv), (r, Value::Set(rows))] if k == "key" && r == "rows" => {
                    let len = path.len();
                    path.push_str(".key");
                    self.at(path, kv, key)?;
                    path.truncate(len);
                    for (i, x) in rows.iter().enumerate() {
                        path.push_str(".rows");
                        self.elem(path, i, x, elem)?;
                        path.truncate(len);
                    }
                    Ok(())
                }
                _ => fail("a group is a record of its key and its rows"),
            },
            (Ty::Fn(..), Value::Fn(_)) => Ok(()),
            (Ty::Param(_), _) => fail("a type parameter has no values of its own"),
            (Ty::Change, _) => fail("a change set is not a value"),
            _ => fail("its shape is of another type"),
        }
    }

    fn elem(&self, path: &mut String, i: usize, x: &Value, elem: &Ty) -> Result<(), Mismatch> {
        let len = path.len();
        path.push_str(&format!("[{i}]"));
        self.at(path, x, elem)?;
        path.truncate(len);
        Ok(())
    }

    /// Judges a value of a declared sum or record, restricted to `ctors` when given.
    fn data(
        &self,
        path: &mut String,
        value: &Value,
        ty: &Ty,
        ctors: Option<&CtorSet>,
    ) -> Result<(), Mismatch> {
        let Ty::Data { def, args, .. } = ty else {
            unreachable!("`data` judges a declared type")
        };
        let d = self.schema.ty(*def);
        let Value::Ctor(c, fs) = value else {
            return Err(self.mismatch(path, value, ty, "its shape is of another type"));
        };
        let Some(ctor) = d.ctors().get(*c as usize) else {
            return Err(self.mismatch(path, value, ty, "its constructor is not one of the type's"));
        };
        if ctors.is_some_and(|s| !s.contains(c)) {
            return Err(self.mismatch(
                path,
                value,
                ty,
                "its constructor is outside the type's constructor set",
            ));
        }
        let want: Vec<(String, Ty)> = d
            .header()
            .iter()
            .chain(&ctor.fields)
            .map(|f| (f.name.clone(), f.ty.subst(args)))
            .collect();
        let step = if d.ctors().len() == 1 {
            String::new()
        } else {
            format!("#{}", ctor.name)
        };
        self.fields(path, &step, value, ty, fs, &want)
    }

    fn fields(
        &self,
        path: &mut String,
        step: &str,
        value: &Value,
        ty: &Ty,
        got: &[(String, Value)],
        want: &[(String, Ty)],
    ) -> Result<(), Mismatch> {
        let names = |fs: &mut dyn Iterator<Item = &String>| {
            fs.map(String::as_str).collect::<Vec<_>>().join(", ")
        };
        if got.len() != want.len() || got.iter().zip(want).any(|((g, _), (w, _))| g != w) {
            let why = format!(
                "its fields are ({}), not ({})",
                names(&mut got.iter().map(|(n, _)| n)),
                names(&mut want.iter().map(|(n, _)| n)),
            );
            return Err(self.mismatch(path, value, ty, why));
        }
        let len = path.len();
        for ((name, v), (_, t)) in got.iter().zip(want) {
            path.push_str(&format!("{step}.{name}"));
            self.at(path, v, t)?;
            path.truncate(len);
        }
        Ok(())
    }

    fn key(
        &self,
        path: &mut String,
        value: &Value,
        ty: &Ty,
        rel: RelId,
        ctors: &Option<CtorSet>,
        k: &Value,
    ) -> Result<(), Mismatch> {
        let r = self.schema.relation(rel);
        let len = path.len();
        path.push_str(".key");
        match r.key.as_slice() {
            [f] => self.at(path, k, &key_field_ty(self.schema, rel, f))?,
            fs => {
                let mut names: Vec<&String> = fs.iter().collect();
                names.sort();
                let want: Ty = Ty::Record(
                    names
                        .into_iter()
                        .map(|f| (f.clone(), key_field_ty(self.schema, rel, f)))
                        .collect(),
                );
                self.at(path, k, &want)?;
            }
        }
        path.truncate(len);
        let Some(State { program, db }) = self.state else {
            return Ok(());
        };
        let all = self.schema.ty(r.elem_def).ctors().len() as u32;
        let narrow = ctors.as_ref().is_some_and(|s| s.len() < all as usize);
        if !(self.stored || narrow) {
            return Ok(());
        }
        let set: CtorSet = ctors.clone().unwrap_or_else(|| (0..all).collect());
        let it = Interpreter::new(self.schema, program, db);
        let names = set.iter().any(|c| {
            db.get(&(rel, *c))
                .is_some_and(|rows| rows.iter().any(|row| &it.key_of(rel, row) == value))
        });
        if names {
            Ok(())
        } else {
            Err(self.mismatch(
                path,
                value,
                ty,
                "it names no row of its constructor set in the database",
            ))
        }
    }

    /// Judges the domain of the newtype `ty` on `value`, when the judgement is of a stored row.
    fn domain(&self, path: &str, value: &Value, ty: &Ty) -> Result<(), Mismatch> {
        let (Some(State { program, db }), true, Ty::Data { def, .. }) =
            (self.state, self.stored, ty)
        else {
            return Ok(());
        };
        let Some((var, p)) = program.domains.get(def) else {
            return Ok(());
        };
        let held = Interpreter::new(self.schema, program, db).eval(
            &program.terms,
            *p,
            &[(*var, value.clone())],
        );
        match held.as_ref().map(Value::as_bool) {
            Ok(Some(true)) => Ok(()),
            Ok(Some(false)) => Err(self.mismatch(path, value, ty, "it is outside the domain")),
            Ok(None) => Err(self.mismatch(path, value, ty, "the domain is not a `Bool`")),
            Err(a) => Err(self.mismatch(path, value, ty, format!("the domain aborts: {a:?}"))),
        }
    }
}

/// Returns why `v` is not a value of base type `b`, if it is not.
fn base(b: Base, v: &Value) -> Result<(), &'static str> {
    let int = |x: i64, bits: u32| {
        let half = 1i128 << (bits - 1);
        if (-half..half).contains(&i128::from(x)) {
            Ok(())
        } else {
            Err("it is out of the range of its width")
        }
    };
    match (b, v) {
        (Base::I8, Value::Int(x)) => int(*x, 8),
        (Base::I16, Value::Int(x)) => int(*x, 16),
        (Base::I32, Value::Int(x)) => int(*x, 32),
        (Base::Decimal(s), Value::Decimal(d, t)) => {
            if s != *t {
                Err("its scale is not the type's")
            } else if d.unsigned_abs() >= 10u128.pow(38) {
                Err("its unscaled value has more than 38 digits")
            } else {
                Ok(())
            }
        }
        (Base::F32, Value::F32(f)) if f.is_nan() && f.to_bits() != f32::NAN.to_bits() => {
            Err("a NaN other than the one NaN")
        }
        (Base::F64, Value::F64(f)) if f.is_nan() && f.to_bits() != f64::NAN.to_bits() => {
            Err("a NaN other than the one NaN")
        }
        (Base::F32, Value::F32(f)) if f.to_bits() == (-0.0f32).to_bits() => Err("it is -0.0"),
        (Base::F64, Value::F64(f)) if f.to_bits() == (-0.0f64).to_bits() => Err("it is -0.0"),
        (Base::Text, Value::Text(t)) if !is_nfc(t) => Err("it is not in NFC"),
        (Base::Date, Value::Date(d)) if d.checked_mul(86_400_000_000).is_none() => {
            Err("it is outside the range of `Instant`")
        }
        (Base::Time, Value::Time(t)) if !(0..86_400_000_000).contains(t) => {
            Err("it is outside a day")
        }
        (Base::ZonedDateTime, Value::Zoned(_, zone)) if !<Value as Textual>::zone_exists(zone) => {
            Err("its zone is not an IANA zone")
        }
        (Base::I64, Value::Int(_))
        | (Base::F32, Value::F32(_))
        | (Base::F64, Value::F64(_))
        | (Base::Text, Value::Text(_))
        | (Base::Bytes, Value::Bytes(_))
        | (Base::SortKey, Value::SortKey(_))
        | (Base::Instant, Value::Instant(_))
        | (Base::Date, Value::Date(_))
        | (Base::Time, Value::Time(_))
        | (Base::DateTime, Value::DateTime(_))
        | (Base::ZonedDateTime, Value::Zoned(..))
        | (Base::Duration, Value::Duration(_))
        | (Base::Period, Value::Period { .. }) => Ok(()),
        _ => Err("its shape is of another type"),
    }
}
