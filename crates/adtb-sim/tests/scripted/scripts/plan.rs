//! Compare the plan document of an explain with the one a `plan` step states.

use adtb::Value;
use adtb_lang::textual::Form;
use serde_json::Value as Json;

use super::read::Step;
use super::role::Results;
use super::session::Differs;

/// A member where the plans differ, with the expected and actual JSON.
type Miss = (String, String, String);

const TOP: [&str; 4] = ["format", "root", "change", "constraints"];
const NODE: [&str; 4] = ["arms", "pruned", "actual", "children"];
const ARM: [&str; 3] = ["ctor", "name", "child"];
const PRUNED: [&str; 3] = ["ctor", "name", "stage"];
const ACTUAL: [&str; 4] = ["rows", "loops", "time_us", "storage"];

/// Compare the plan document of the one result of the step's body with the step's rows.
pub(super) fn compare(step: &Step, mut results: Results) -> Result<(), Differs> {
    let at = |m: String| Differs::at(step, m);
    let want: Json = serde_json::from_str(&step.rows.join("\n"))
        .map_err(|e| at(format!("the expected document does not read: {e}")))?;
    if want.get("format").is_none() {
        return Err(at("the expected document states no `format`".into()));
    }
    unsound(&want, "").map_err(|(path, why)| at(format!("the expected `{path}` {why}")))?;
    let document = match (results.pop(), results.is_empty()) {
        (Some((form, rows)), true) => document(&form, &rows),
        _ => None,
    };
    let document = document.ok_or_else(|| at("the body has no one explain result".into()))?;
    let got: Json = serde_json::from_str(&document)
        .map_err(|e| at(format!("the plan document does not read: {e}")))?;
    unshaped(&got, "", &["format", "release", "root"])
        .and_then(|()| top(&want, &got))
        .map_err(|(path, e, a)| at(format!("the plan differs at `{path}`")).values(e, a))
}

/// Returns the `document` of `rows` when they are the one row of an explain.
fn document(form: &Form, rows: &[Value]) -> Option<String> {
    let Form::Record(fields) = form else {
        return None;
    };
    let i = fields.iter().position(|(n, _)| n == "document")?;
    match rows {
        [Value::Record(fs)] => match fs.get(i) {
            Some(Value::Text(t)) => Some(t.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn shown(v: Option<&Json>) -> String {
    v.map_or_else(|| "nothing".to_owned(), Json::to_string)
}

fn equal(at: &str, want: &Json, got: &Json) -> Result<(), Miss> {
    match want == got {
        true => Ok(()),
        false => Err((at.to_owned(), want.to_string(), got.to_string())),
    }
}

/// Fail naming the first of `names` that `v` lacks, then check each node `v` holds.
fn unshaped(v: &Json, at: &str, names: &[&str]) -> Result<(), Miss> {
    if let Some(m) = names.iter().find(|m| v.get(m).is_none()) {
        return Err((
            format!("{at}.{m}"),
            format!("a member `{m}`"),
            v.to_string(),
        ));
    }
    let each = |member: &str, names: &[&str]| -> Result<(), Miss> {
        let at = format!("{at}.{member}");
        match v.get(member) {
            Some(Json::Array(xs)) => xs
                .iter()
                .enumerate()
                .try_for_each(|(i, x)| unshaped(x, &format!("{at}[{i}]"), names)),
            Some(x) if member == "actual" || member == "root" || member == "child" => {
                unshaped(x, &at, names)
            }
            Some(x) if matches!(member, "children" | "arms" | "pruned") => {
                Err((at, "an array".into(), x.to_string()))
            }
            _ => Ok(()),
        }
    };
    if let Some(s) = v.get("stage").filter(|s| *s != "compile" && *s != "start") {
        return Err((
            format!("{at}.stage"),
            "`compile` or `start`".into(),
            s.to_string(),
        ));
    }
    let node = ["kind", "estimate", "children"];
    each("root", &node)?;
    each("child", &node)?;
    each("children", &node)?;
    each("arms", &ARM)?;
    each("pruned", &PRUNED)?;
    each("actual", &ACTUAL)
}

/// Check each member of `want` that `known` names against the same member of `got` with
/// `each`; any other member differs.
fn members(
    at: &str,
    want: &Json,
    got: &Json,
    known: &[&str],
    each: impl Fn(&str, &str, &Json, &Json) -> Result<(), Miss>,
) -> Result<(), Miss> {
    let Json::Object(ms) = want else {
        return Err((at.to_owned(), want.to_string(), got.to_string()));
    };
    for (name, w) in ms {
        let at = format!("{at}.{name}");
        if !known.contains(&name.as_str()) {
            return Err((at, w.to_string(), "a member no plan step compares".into()));
        }
        match got.get(name) {
            Some(g) => each(&at, name, w, g)?,
            None => return Err((at, w.to_string(), shown(None))),
        }
    }
    Ok(())
}

fn top(want: &Json, got: &Json) -> Result<(), Miss> {
    members("", want, got, &TOP, |at, name, w, g| match name {
        "root" => node(at, w, g),
        _ => equal(at, w, g),
    })
}

/// Match node `want` against `got`: its arms and pruned constructors as sets by `name`, the
/// rows and loops of its actual, and each of its children against a node below `got`, the
/// first in preorder after the one the child before matched. A child no node matches differs
/// where the candidate that matched furthest differs.
fn node(at: &str, want: &Json, got: &Json) -> Result<(), Miss> {
    members(at, want, got, &NODE, |at, name, w, g| match name {
        "arms" | "pruned" => {
            let (ws, gs) = (array(w), array(g));
            if ws.len() != gs.len() {
                return equal(at, w, g);
            }
            let known: &[&str] = if name == "arms" { &ARM } else { &PRUNED };
            ws.iter().try_for_each(|w| {
                let at = format!("{at}[{}]", w["name"].as_str().unwrap_or_default());
                match gs.iter().find(|g| g.get("name") == w.get("name")) {
                    Some(g) => members(&at, w, g, known, |at, name, w, g| match name {
                        "child" => node(at, w, g),
                        _ => equal(at, w, g),
                    }),
                    None => Err((at, w.to_string(), shown(None))),
                }
            })
        }
        "actual" => members(at, w, g, &ACTUAL[..2], |at, _, w, g| equal(at, w, g)),
        _ => {
            let below = descendants(got);
            let rank = |m: &Miss| (m.0.len(), m.2 != shown(None));
            let mut from = 0;
            for (i, w) in array(w).iter().enumerate() {
                let at = format!("{at}[{i}]");
                let mut deepest = (at.clone(), w.to_string(), "no node below matches".into());
                let found = (from..below.len()).find(|&j| match node(&at, w, below[j]) {
                    Ok(()) => true,
                    Err(miss) => {
                        if rank(&miss) > rank(&deepest) {
                            deepest = miss;
                        }
                        false
                    }
                });
                from = found.ok_or(deepest)? + 1;
            }
            Ok(())
        }
    })
}

fn array(v: &Json) -> &[Json] {
    v.as_array().map_or(&[], Vec::as_slice)
}

/// Fail naming where expected node `want` holds a value that is no array where one belongs, an
/// empty `children` or `actual`, or an arm or pruned entry without a `name` or with a repeated one.
fn unsound(want: &Json, at: &str) -> Result<(), (String, &'static str)> {
    for (m, w) in want.as_object().into_iter().flatten() {
        let at = format!("{at}.{m}");
        let named = |x: &Json| x.get("name").and_then(Json::as_str).map(str::to_owned);
        let names: Option<Vec<String>> = array(w).iter().map(named).collect();
        let why = match m.as_str() {
            "children" | "arms" | "pruned" if !w.is_array() => "is no array",
            "actual" if w.as_object().is_some_and(serde_json::Map::is_empty) => "compares nothing",
            "children" if array(w).is_empty() => "compares nothing",
            "arms" | "pruned" if names.is_none() => "holds an entry with no `name`",
            "arms" | "pruned"
                if names
                    .as_ref()
                    .is_some_and(|n| (1..n.len()).any(|i| n[..i].contains(&n[i]))) =>
            {
                "names an entry twice"
            }
            _ => {
                let mut nodes = array(w).iter().enumerate();
                match m.as_str() {
                    "root" => unsound(w, &at)?,
                    "children" => nodes.try_for_each(|(i, x)| unsound(x, &format!("{at}[{i}]")))?,
                    "arms" => nodes.try_for_each(|(_, x)| {
                        let name = x["name"].as_str().unwrap_or_default();
                        unsound(&x["child"], &format!("{at}[{name}].child"))
                    })?,
                    _ => {}
                }
                continue;
            }
        };
        return Err((at, why));
    }
    Ok(())
}

/// Returns the nodes below `node` in preorder, through children and the child of each arm.
fn descendants(node: &Json) -> Vec<&Json> {
    let mut out = Vec::new();
    let arms = node
        .get("arms")
        .and_then(Json::as_array)
        .into_iter()
        .flatten();
    let children = node
        .get("children")
        .and_then(Json::as_array)
        .into_iter()
        .flatten();
    for c in children.chain(arms.filter_map(|a| a.get("child"))) {
        out.push(c);
        out.extend(descendants(c));
    }
    out
}
