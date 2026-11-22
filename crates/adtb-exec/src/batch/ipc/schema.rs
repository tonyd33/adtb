use super::flat::{Reader, Tab, Table, Val};
use super::{FileCtor, FileSchema, Form, IpcError, form, union_kids};
use crate::key::Scalar;

/// How a stream's field tree maps onto the expected one.
#[derive(Debug, Default)]
pub(super) struct Plan {
    /// The dictionary id of an enum field.
    pub dict: Option<i64>,
    /// The stream's children in its order, each with its position among the expected ones.
    pub kids: Vec<(usize, Plan)>,
}

/// Returns `f` with an `Option`'s present column resolved to its nullable inner form.
pub(super) fn resolve(f: Form<'_>, nullable: bool) -> (Form<'_>, bool) {
    match f {
        Form::Nullable(inner) => (form(inner), true),
        f => (f, nullable),
    }
}

/// Returns the children of a field of form `f`, each with its name.
pub(super) fn kids(f: Form<'_>) -> Vec<(String, Form<'_>)> {
    match f {
        Form::Struct { fields, .. } => fields
            .iter()
            .map(|x| (x.name.clone(), form(&x.ty)))
            .collect(),
        Form::List(e) => vec![("item".into(), form(e))],
        Form::Union { ctors, base } => union_kids(ctors, base),
        _ => Vec::new(),
    }
}

fn int(bits: i64, signed: bool) -> (i64, Table) {
    (
        2,
        Table::default()
            .num(0, 4, bits, 0)
            .num(1, 1, signed.into(), 0),
    )
}

const MICROS: i64 = 2;

fn scalar(s: Scalar) -> (i64, Table) {
    let t = Table::default();
    match s {
        Scalar::I8 => int(8, true),
        Scalar::I16 => int(16, true),
        Scalar::I32 => int(32, true),
        Scalar::I64 => int(64, true),
        Scalar::F32 => (3, t.num(0, 2, 1, 0)),
        Scalar::F64 => (3, t.num(0, 2, 2, 0)),
        Scalar::Decimal(s) => (
            7,
            t.num(0, 4, 38, 0)
                .num(1, 4, s.into(), 0)
                .num(2, 4, 128, 128),
        ),
        Scalar::Text | Scalar::Period | Scalar::ZonedDateTime => (5, t),
        Scalar::Bytes | Scalar::SortKey => (4, t),
        Scalar::Instant => (10, t.num(0, 2, MICROS, 0).with(1, Val::Str("UTC".into()))),
        Scalar::DateTime => (10, t.num(0, 2, MICROS, 0)),
        Scalar::Date => (8, t.num(0, 2, 0, 1)),
        Scalar::Time => (9, t.num(0, 2, MICROS, 1).num(1, 4, 64, 32)),
        Scalar::Duration => (18, t.num(0, 2, MICROS, 1)),
    }
}

/// Returns the Field table of `f` without its children.
fn head(name: &str, f: Form<'_>, nullable: bool, dict: i64) -> Table {
    let (tt, ty) = match f {
        Form::Scalar(s) => scalar(s),
        Form::Bool => (6, Table::default()),
        Form::Enum(_) => (5, Table::default()),
        Form::Struct { .. } => (13, Table::default()),
        Form::List(_) => (12, Table::default()),
        Form::Union { ctors, .. } => {
            let ids = (0..union_kids(ctors, 0).len() as i32).collect();
            (14, Table::default().num(0, 2, 1, 0).with(1, Val::I32s(ids)))
        }
        Form::Nullable(_) => unreachable!("a nullable form is resolved before its head"),
    };
    let t = Table::default()
        .with(0, Val::Str(name.into()))
        .num(1, 1, nullable.into(), 0)
        .num(2, 1, tt, 0)
        .with(3, Val::Table(ty));
    if !matches!(f, Form::Enum(_)) {
        return t;
    }
    let index = Table::default().num(0, 4, 32, 0).num(1, 1, 1, 0);
    let d = Table::default()
        .num(0, 8, dict, 0)
        .with(1, Val::Table(index))
        .num(2, 1, 0, 0)
        .num(3, 2, 0, 0);
    t.with(4, Val::Table(d))
}

fn field<'a>(name: &str, f: Form<'a>, nullable: bool, dicts: &mut Vec<&'a [FileCtor]>) -> Table {
    let (f, nullable) = resolve(f, nullable);
    let t = head(name, f, nullable, dicts.len() as i64);
    if let Form::Enum(cs) = f {
        dicts.push(cs);
    }
    let kids = kids(f)
        .into_iter()
        .map(|(n, k)| field(&n, k, false, dicts))
        .collect();
    t.with(5, Val::Tables(kids))
}

/// Returns the Schema table of `file`, and the constructors of each enum field in the order of
/// their dictionary ids.
pub(super) fn schema(file: &FileSchema) -> (Table, Vec<&[FileCtor]>) {
    let mut dicts = Vec::new();
    let fields = file
        .fields
        .iter()
        .map(|f| field(&f.name, form(&f.ty), false, &mut dicts))
        .collect();
    (
        Table::default()
            .num(0, 2, 0, 0)
            .with(1, Val::Tables(fields)),
        dicts,
    )
}

fn same_slot(r: Reader<'_>, got: Tab, slot: u16, v: &Val) -> Result<bool, IpcError> {
    Ok(match v {
        Val::Num { width, v, default } => r.num(got, slot, *width, *default)? == *v,
        Val::Str(s) => r.str(got, slot)? == Some(s.as_str()),
        Val::I32s(xs) if !r.has(got, slot)? => xs.iter().copied().eq(0..xs.len() as i32),
        Val::I32s(xs) => r
            .nums(got, slot, 4)?
            .into_iter()
            .eq(xs.iter().map(|x| i64::from(*x))),
        Val::Table(t) => match r.table(got, slot)? {
            Some(g) => same(r, g, t, None)?,
            None => false,
        },
        Val::Tables(_) | Val::Pairs(_) => unreachable!("a type table holds no vector of tables"),
    })
}

/// Returns whether table `got` holds `want`'s fields and no other, ignoring slot `skip`.
fn same(r: Reader<'_>, got: Tab, want: &Table, skip: Option<u16>) -> Result<bool, IpcError> {
    let top = want.0.iter().map(|(s, _)| s + 1).max().unwrap_or(0);
    for slot in 0..r.slots(got)?.max(top) {
        let ok = match want.0.iter().find(|(s, _)| *s == slot) {
            _ if Some(slot) == skip => true,
            Some((_, v)) => same_slot(r, got, slot, v)?,
            None => !r.has(got, slot)?,
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Returns how the stream's field `got` maps onto the expected field `name` of form `f`.
///
/// Fail at the first difference, before reading the stream's children past the expected ones.
pub(super) fn check(
    r: Reader<'_>,
    got: Tab,
    (name, f): (&str, Form<'_>),
    nullable: bool,
    path: &str,
) -> Result<Plan, IpcError> {
    let (f, nullable) = resolve(f, nullable);
    let path = if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}.{name}")
    };
    let differs = |what| IpcError::SchemaDiffers {
        path: path.clone(),
        what,
    };
    let want = head(name, f, nullable, 0);
    for (slot, v) in &want.0 {
        let what = ["name", "nullability", "type", "type", "dictionary"][usize::from(*slot)];
        let ok = match (slot, v) {
            (1, _) => true,
            (4, Val::Table(d)) => match r.table(got, 4)? {
                Some(g) => same(r, g, d, Some(0))?,
                None => false,
            },
            _ => same_slot(r, got, *slot, v)?,
        };
        if !ok {
            return Err(differs(what));
        }
    }
    let dict = match (f, r.table(got, 4)?) {
        (Form::Enum(_), Some(d)) => Some(r.num(d, 0, 8, 0)?),
        (_, None) => None,
        (_, Some(_)) => return Err(differs("dictionary")),
    };
    let want = kids(f);
    let got = r.tables(got, 5)?;
    if want.len() != got.len() {
        return Err(differs("children"));
    }
    let mut used = vec![false; want.len()];
    let mut plan = Plan {
        dict,
        kids: Vec::new(),
    };
    for (i, g) in got.into_iter().enumerate() {
        let j = match f {
            Form::Union { .. } => {
                let n = r.str(g, 0)?;
                want.iter().position(|(w, _)| Some(w.as_str()) == n)
            }
            _ => Some(i),
        };
        let j = j.filter(|j| !used[*j]).ok_or_else(|| differs("children"))?;
        used[j] = true;
        let (n, k) = &want[j];
        plan.kids.push((j, check(r, g, (n, *k), false, &path)?));
    }
    Ok(plan)
}
