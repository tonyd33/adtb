use super::*;

/// The running schema, its program and ad hoc parameters `p` and `q` of any types, checked once.
struct Tables {
    compiled: Compiled,
}

impl Tables {
    fn new() -> Tables {
        Tables::with("")
    }

    fn with(extra: &str) -> Tables {
        let schema = schema(extra).expect("the schema builds");
        let compiled = check_schema(schema).expect("the schema checks");
        Tables { compiled }
    }

    fn ty(&self, t: &str) -> Ty {
        let parsed = parse_type(t, Options::default()).ok().expect("a type");
        Checker::of_adhoc(
            self.compiled.committed(),
            Visible::all(self.compiled.committed()),
        )
        .resolve_type(&parsed, FnTypes::Allowed)
        .expect("a known type")
    }

    /// Returns the outcome of checking `text` with each parameter at its written type: `Ok` or the
    /// first refusal.
    fn check(&self, params: &[(&str, &str)], text: &str) -> Result<(), String> {
        let tys = params
            .iter()
            .map(|(n, t)| (n.to_string(), self.ty(t)))
            .collect();
        let items = parse_adhoc(text, Options::default()).ok().expect("parsed");
        check_adhoc(&self.compiled, &items, &tys)
            .map(drop)
            .map_err(|ds| ds[0].message.clone())
    }
}

const NUMERIC: [&str; 9] = [
    "I8",
    "I16",
    "I32",
    "I64",
    "Decimal 0",
    "Decimal 2",
    "Decimal 20",
    "F32",
    "F64",
];

/// Returns the inclusive range of integer type `t`, `None` for a type that is not an integer.
fn int_range(t: &str) -> Option<(i128, i128)> {
    let bits = match t {
        "I8" => 8,
        "I16" => 16,
        "I32" => 32,
        "I64" => 64,
        _ => return None,
    };
    Some((-(1 << (bits - 1)), (1 << (bits - 1)) - 1))
}

fn scale(t: &str) -> Option<u32> {
    t.strip_prefix("Decimal ")
        .map(|s| s.parse().expect("a scale"))
}

/// Returns a literal's unscaled value and its number of fractional digits.
fn literal_value(lit: &str) -> (i128, u32) {
    let (whole, frac) = lit.split_once('.').unwrap_or((lit, ""));
    let digits = format!("{whole}{frac}");
    (
        digits.parse().expect("a literal fits i128"),
        frac.len() as u32,
    )
}

/// Returns the verdict of literal `lit` checked against numeric type `t`.
fn literal_verdict(lit: &str, t: &str) -> Result<(), String> {
    let (value, k) = literal_value(lit);
    let out_of_range = || Err(format!("literal out of range of {t}"));
    if let Some((lo, hi)) = int_range(t) {
        if k > 0 {
            return Err(format!("a fractional literal does not check against {t}"));
        }
        return if (lo..=hi).contains(&value) {
            Ok(())
        } else {
            out_of_range()
        };
    }
    if let Some(s) = scale(t) {
        if k > s {
            return Err(format!(
                "a literal with {k} fractional digits does not check against {t}"
            ));
        }
        let unscaled = value * 10i128.pow(s - k);
        return if unscaled.unsigned_abs() < 10u128.pow(38) {
            Ok(())
        } else {
            out_of_range()
        };
    }
    Ok(())
}

const LITERALS: [&str; 24] = [
    "0",
    "127",
    "128",
    "-128",
    "-129",
    "32767",
    "32768",
    "-32768",
    "-32769",
    "2147483647",
    "2147483648",
    "-2147483648",
    "-2147483649",
    "9223372036854775807",
    "9223372036854775808",
    "-9223372036854775808",
    "-9223372036854775809",
    "999999999999999999",
    "1000000000000000000",
    "99999999999999999999999999999999999999",
    "1.5",
    "-0.25",
    "0.125",
    "999999999999999999.99",
];

#[test]
fn a_numeric_literal_takes_the_other_operands_type_within_its_range() {
    let t = Tables::new();
    let mut differ = Vec::new();
    let mut accepted = 0;
    for ty in NUMERIC {
        for lit in LITERALS {
            for text in [format!("p == {lit}"), format!("{lit} == p")] {
                let want = if scale(ty).is_some() {
                    let (_, k) = literal_value(lit);
                    literal_verdict(lit, &format!("Decimal {k}"))
                } else {
                    literal_verdict(lit, ty)
                };
                let got = t.check(&[("p", ty)], &text);
                accepted += usize::from(got.is_ok());
                if got != want {
                    differ.push(format!("p: {ty}, {text}: {got:?}, expected {want:?}"));
                }
            }
        }
    }
    assert_eq!(Vec::<String>::new(), differ);
    assert!(accepted > NUMERIC.len() * 2, "only {accepted} accepted");
}

#[test]
fn a_literal_with_no_expected_type_is_i64_or_decimal_of_its_own_scale() {
    let t = Tables::new();
    let mut differ = Vec::new();
    for lit in LITERALS {
        let (_, k) = literal_value(lit);
        let want = if k == 0 {
            literal_verdict(lit, "I64")
        } else {
            literal_verdict(lit, &format!("Decimal {k}"))
        };
        let got = t.check(&[], lit);
        if got != want {
            differ.push(format!("{lit}: {got:?}, expected {want:?}"));
        }
    }
    assert_eq!(Vec::<String>::new(), differ);
}

/// Returns the verdict of `p op q` with `p: a` and `q: b`.
fn arithmetic_verdict(op: &str, a: &str, b: &str) -> Result<(), String> {
    let refused = || Err(format!("`{op}` does not apply to {a} and {b}"));
    match (scale(a), scale(b)) {
        (Some(s), Some(t)) => match op {
            "+" | "-" => Ok(()),
            "*" if s + t <= 38 => Ok(()),
            "*" => Err(format!(
                "`*` of `{a}` and `{b}` has scale {} above 38; rescale an operand first",
                s + t
            )),
            _ => {
                Err("decimals have no `/` and no `%`; use `div` with a scale and a rounding".into())
            }
        },
        _ if a != b => refused(),
        _ if int_range(a).is_some() => Ok(()),
        _ if op == "%" => refused(),
        _ => Ok(()),
    }
}

#[test]
fn arithmetic_takes_operands_of_one_numeric_type() {
    let t = Tables::new();
    let mut differ = Vec::new();
    let mut accepted = 0;
    for a in NUMERIC {
        for b in NUMERIC {
            for op in ["+", "-", "*", "/", "%"] {
                let text = format!("p {op} q");
                let got = t.check(&[("p", a), ("q", b)], &text);
                accepted += usize::from(got.is_ok());
                let want = arithmetic_verdict(op, a, b);
                if got != want {
                    differ.push(format!(
                        "p: {a}, q: {b}, {text}: {got:?}, expected {want:?}"
                    ));
                }
            }
        }
    }
    assert_eq!(Vec::<String>::new(), differ);
    assert!(accepted > NUMERIC.len() * 3, "only {accepted} accepted");
}

#[derive(Clone, Copy, PartialEq)]
enum Site {
    /// A value with no position that omits a field.
    Plain,
    /// The row of an `insert` or the result of its source query's `select`.
    InsertRow,
    /// The value after `set` in an `update`, or an arm of a `match` directly after it.
    UpdateSet,
}

/// Returns the verdict of `Identity.Google` given `fields`, among `id`, `user`, `gid` and `sub`,
/// at `site`.
fn construction_verdict(site: Site, fields: &[&str]) -> Result<(), String> {
    if fields.contains(&"sub") {
        return Err("`Google` has no field `sub`".into());
    }
    if site == Site::UpdateSet && fields.contains(&"id") {
        return Err(
            "the new value keeps the row's key: `id` is a key field of `identities` and is not given"
                .into(),
        );
    }
    let omittable: &[&str] = match site {
        Site::Plain => &[],
        Site::InsertRow => &["id"],
        Site::UpdateSet => &["id", "user"],
    };
    let missing = ["id", "user", "gid"]
        .into_iter()
        .find(|f| !fields.contains(f) && !omittable.contains(f));
    match (missing, site) {
        (Some(f), Site::Plain) => Err(format!("field `{f}` of `Google` is missing")),
        (Some(f), _) => Err(format!(
            "field `{f}` of `Google` is missing, and `identities` has no default for it"
        )),
        (None, _) => Ok(()),
    }
}

#[test]
fn a_constructor_gives_every_field_except_those_its_position_fills() {
    let t = Tables::new();
    let params = [("u", "Key users"), ("i", "Key identities")];
    let sites = [
        (Site::Plain, "[Google { F }]"),
        (Site::Plain, "[Identity.Google { F }]"),
        (Site::Plain, "insert identities from [Google { F }]"),
        (Site::InsertRow, "insert identities.Google { F }"),
        (
            Site::InsertRow,
            "insert identities from (from x in users select Google { F })",
        ),
        (
            Site::InsertRow,
            "insert identities from (from x in users select match true | true -> Google { F } | false -> Google { F })",
        ),
        (
            Site::UpdateSet,
            "update r in identities.Apple set Google { F }",
        ),
        (
            Site::UpdateSet,
            "update r in identities set match r | EmailPass e -> Google { F } | Google g -> Google { F } | Apple a -> Google { F }",
        ),
    ];
    let values = [
        ("id", "i"),
        ("user", "u"),
        ("gid", "\"g\""),
        ("sub", "\"s\""),
    ];
    let mut differ = Vec::new();
    let mut accepted = vec![0; sites.len()];
    for (n, (site, text)) in sites.iter().enumerate() {
        for given in 0..16u32 {
            let fields: Vec<&str> = (0..4)
                .filter(|b| given & (1 << b) != 0)
                .map(|b| values[b].0)
                .collect();
            let written: Vec<String> = (0..4)
                .filter(|b| given & (1 << b) != 0)
                .map(|b| format!("{}: {}", values[b].0, values[b].1))
                .collect();
            let text = text.replace('F', &written.join(", "));
            let got = t.check(&params, &text);
            accepted[n] += usize::from(got.is_ok());
            let want = construction_verdict(*site, &fields);
            if got != want {
                differ.push(format!("{text}: {got:?}, expected {want:?}"));
            }
        }
    }
    assert_eq!(Vec::<String>::new(), differ);
    assert!(
        accepted.iter().all(|&n| n > 0),
        "a site accepted nothing: {accepted:?}"
    );
}

#[test]
fn a_constructor_with_no_expected_type_names_one_declared_sum() {
    let t = Tables::with(
        "type Letter = Sent | Kept { at: Instant }\nfn kept (l: Letter) -> Bool = match l | Sent -> false | Kept _ -> true",
    );
    const AMBIGUOUS: &str =
        "`Sent` is a constructor of `Letter` and `Status`; qualify it as `T.Sent`";
    let cases = [
        ("[Draft]", Ok(())),
        ("[Kept { at: p }]", Ok(())),
        ("[Sent { at: p }]", Err(AMBIGUOUS)),
        ("[Sent]", Err(AMBIGUOUS)),
        ("[Status.Sent { at: p }]", Ok(())),
        ("[Letter.Sent]", Ok(())),
        ("kept(Sent)", Ok(())),
    ];
    let mut differ = Vec::new();
    for (text, want) in cases {
        let got = t.check(&[("p", "Instant")], text);
        let want = want.map_err(str::to_string);
        if got != want {
            differ.push(format!("{text}: {got:?}, expected {want:?}"));
        }
    }
    assert_eq!(Vec::<String>::new(), differ);
}
