use adtb_reference::{Abort, Fault, Value};
use adtb_testkit::fixture::Fixture;

use super::shared::{bindings, parse};
use super::{Fields, finish, take};

enum Mode {
    Eval,
    Call,
}

enum Expected {
    Value(Value),
    Fault(Fault),
}

fn fault(text: &str) -> Fault {
    match text {
        "DivisionByZero" => Fault::DivisionByZero,
        "OutOfRange" => Fault::OutOfRange,
        _ => panic!("unknown typed fault {text}"),
    }
}

fn raw(fixture: &Fixture, probe: &str, line: &str) -> Value {
    let mut cells = line.split('|');
    let ctor = cells
        .next()
        .expect("a raw outcome has a constructor")
        .parse()
        .expect("raw constructor is u32");
    let value = match probe {
        "login" => {
            let field = cells.next().expect("login has a field");
            let value = parse(fixture, "Text", cells.next().expect("login has a payload"));
            Value::Ctor(ctor, vec![(field.into(), value)])
        }
        "tags" => {
            let count = cells
                .next()
                .expect("tag has a count")
                .parse()
                .expect("count is I64");
            Value::record(vec![
                ("kind", Value::Ctor(ctor, Vec::new())),
                ("n", Value::Int(count)),
            ])
        }
        "try" => {
            let key = cells.next().expect("try has a key");
            let kind = cells.next().expect("try names its payload");
            let payload = cells.next().expect("try has a payload");
            let (field, value) = match kind {
                "ok" => ("value", parse(fixture, "I32", payload)),
                "fault" => ("error", fixture.fault(fault(payload))),
                _ => panic!("unknown try payload {kind}"),
            };
            Value::record(vec![
                ("line", parse(fixture, "Key raws", key)),
                ("n", Value::Ctor(ctor, vec![(field.into(), value)])),
            ])
        }
        "anonymous" => {
            let payload = cells.next().expect("anonymous outcome has a payload cell");
            let fields = if payload.is_empty() {
                Vec::new()
            } else {
                vec![("n".into(), parse(fixture, "I64", payload))]
            };
            Value::Ctor(ctor, fields)
        }
        _ => panic!("unknown raw assertion {probe}"),
    };
    assert!(cells.next().is_none(), "unused raw outcome cells");
    value
}

fn expected(fixture: &Fixture, fields: &mut Fields) -> Expected {
    if let Some(name) = fields.remove("fault") {
        return Expected::Fault(fault(&name));
    }
    let shape = take(fields, "shape");
    let rows = take(fields, "expected");
    let values: Vec<_> = if let Some(probe) = fields.remove("raw") {
        rows.lines()
            .map(|line| raw(fixture, &probe, line))
            .collect()
    } else {
        let ty = take(fields, "type");
        rows.lines().map(|line| parse(fixture, &ty, line)).collect()
    };
    Expected::Value(match shape.as_str() {
        "set" => Value::set(values),
        "list" => Value::List(values),
        "scalar" => {
            let [value]: [Value; 1] = values
                .try_into()
                .expect("a scalar expectation has one value");
            value
        }
        _ => panic!("unknown outcome shape {shape}"),
    })
}

pub(super) fn run(fixture: &Fixture, mode: &str, mut fields: Fields) {
    let mode = match mode {
        "eval" => Mode::Eval,
        "call" => Mode::Call,
        _ => panic!("unknown mode {mode}"),
    };
    let source = take(&mut fields, "source");
    let want = expected(fixture, &mut fields);
    let got = match mode {
        Mode::Eval => {
            let mut params = bindings(fixture, fields.remove("params"));
            if let Some(raw) = fields.remove("raw-param") {
                let (name, id) = raw
                    .split_once('|')
                    .expect("raw parameter names its constructor");
                let (_, _, value) = params
                    .iter_mut()
                    .find(|(n, _, _)| n == name)
                    .expect("raw parameter is bound");
                let Value::Ctor(ctor, _) = value else {
                    panic!("a raw parameter is a constructor")
                };
                *ctor = id.parse().expect("raw parameter constructor is u32");
            }
            let borrowed: Vec<_> = params
                .iter()
                .map(|(n, t, v)| (n.as_str(), t.as_str(), v.clone()))
                .collect();
            fixture.eval_with(&source, &borrowed)
        }
        Mode::Call => {
            let args = take(&mut fields, "args");
            let values = args
                .lines()
                .map(|line| {
                    let (ty, text) = line
                        .split_once('|')
                        .expect("an argument has type and value");
                    parse(fixture, ty, text)
                })
                .collect();
            fixture.call_query(&source, values)
        }
    };
    finish(&fields);
    match want {
        Expected::Value(value) => assert_eq!(Ok(value), got, "exact typed outcome"),
        Expected::Fault(fault) => assert_eq!(Err(Abort::Fault(fault)), got, "exact typed fault"),
    }
}
