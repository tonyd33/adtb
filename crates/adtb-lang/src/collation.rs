/// The version every collation name this binary supports carries after its `@`.
pub const VERSION: &str = "cldr-49";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    Primary,
    Secondary,
    Tertiary,
    Quaternary,
    Identical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseFirst {
    Upper,
    Lower,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaxVariable {
    Space,
    Punctuation,
    Symbol,
    Currency,
}

/// A collation name read as a BCP 47 language tag with collation keywords, then its version.
/// A keyword the name does not set is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collation {
    /// The language, script and region subtags, joined by `-`.
    pub locale: String,
    pub strength: Option<Strength>,
    /// `Some(true)` for `ka-shifted`, `Some(false)` for `ka-noignore`.
    pub shifted: Option<bool>,
    pub case_level: Option<bool>,
    pub case_first: Option<CaseFirst>,
    pub numeric: Option<bool>,
    pub max_variable: Option<MaxVariable>,
}

fn alpha(s: &str, lens: &[usize]) -> bool {
    lens.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphabetic())
}

fn bool_value(v: &str) -> Option<bool> {
    match v {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Reads collation name `name`, `tag@version`, where `tag` is a language, an optional script and
/// region, and an optional `-u-` extension of the keywords `ks`, `ka`, `kc`, `kf`, `kn` and `kv`,
/// each once.
pub fn parse(name: &str) -> Result<Collation, String> {
    let Some((tag, version)) = name.split_once('@') else {
        return Err(format!(
            "collation `{name}` names no version; write it as `name@version`"
        ));
    };
    if version != VERSION {
        return Err(format!(
            "collation `{name}` names version `{version}`; this database supports `{VERSION}`"
        ));
    }
    let unknown = || format!("collation `{name}` is not a collation this database supports");
    let mut parts = tag.split('-').peekable();
    let language = parts.next().unwrap_or_default();
    if !alpha(language, &[2, 3]) || language.bytes().any(|b| b.is_ascii_uppercase()) {
        return Err(unknown());
    }
    let mut locale = language.to_string();
    if let Some(s) = parts.next_if(|s| alpha(s, &[4])) {
        let mut cs = s.chars();
        let title =
            cs.next().is_some_and(|c| c.is_ascii_uppercase()) && cs.all(|c| c.is_ascii_lowercase());
        if !title {
            return Err(unknown());
        }
        locale = format!("{locale}-{s}");
    }
    if let Some(r) = parts.next_if(|s| {
        (alpha(s, &[2]) && s.bytes().all(|b| b.is_ascii_uppercase()))
            || (s.len() == 3 && s.bytes().all(|b| b.is_ascii_digit()))
    }) {
        locale = format!("{locale}-{r}");
    }
    let mut c = Collation {
        locale,
        strength: None,
        shifted: None,
        case_level: None,
        case_first: None,
        numeric: None,
        max_variable: None,
    };
    match parts.next() {
        None => return Ok(c),
        Some("u") => {}
        Some(_) => return Err(unknown()),
    }
    let mut seen = Vec::new();
    while let Some(key) = parts.next() {
        let value = parts.next().ok_or_else(unknown)?;
        if seen.contains(&key) {
            return Err(unknown());
        }
        seen.push(key);
        let ok = match key {
            "ks" => {
                c.strength = Some(match value {
                    "level1" => Strength::Primary,
                    "level2" => Strength::Secondary,
                    "level3" => Strength::Tertiary,
                    "level4" => Strength::Quaternary,
                    "identic" => Strength::Identical,
                    _ => return Err(unknown()),
                });
                true
            }
            "ka" => {
                c.shifted = match value {
                    "shifted" => Some(true),
                    "noignore" => Some(false),
                    _ => None,
                };
                c.shifted.is_some()
            }
            "kc" => {
                c.case_level = bool_value(value);
                c.case_level.is_some()
            }
            "kn" => {
                c.numeric = bool_value(value);
                c.numeric.is_some()
            }
            "kf" => {
                c.case_first = match value {
                    "upper" => Some(CaseFirst::Upper),
                    "lower" => Some(CaseFirst::Lower),
                    "false" => Some(CaseFirst::Off),
                    _ => None,
                };
                c.case_first.is_some()
            }
            "kv" => {
                c.max_variable = match value {
                    "space" => Some(MaxVariable::Space),
                    "punct" => Some(MaxVariable::Punctuation),
                    "symbol" => Some(MaxVariable::Symbol),
                    "currency" => Some(MaxVariable::Currency),
                    _ => None,
                };
                c.max_variable.is_some()
            }
            _ => false,
        };
        if !ok {
            return Err(unknown());
        }
    }
    if seen.is_empty() {
        return Err(unknown());
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_a_tag_and_the_supported_version() {
        let c = parse("de-u-ks-level2@cldr-49").unwrap();
        assert_eq!("de", c.locale);
        assert_eq!(Some(Strength::Secondary), c.strength);
        let c = parse("zh-Hant-TW@cldr-49").unwrap();
        assert_eq!("zh-Hant-TW", c.locale);
        assert_eq!(None, c.strength);
        let c = parse("en-u-kn-true-ka-shifted-kf-upper@cldr-49").unwrap();
        assert_eq!(Some(true), c.numeric);
        assert_eq!(Some(true), c.shifted);
        assert_eq!(Some(CaseFirst::Upper), c.case_first);
        assert!(parse("und@cldr-49").is_ok());
        assert!(parse("es-419@cldr-49").is_ok());
    }

    #[test]
    fn unversioned_and_unknown_names_are_refused() {
        assert!(parse("de").unwrap_err().contains("names no version"));
        assert!(
            parse("de@icu-74")
                .unwrap_err()
                .contains("supports `cldr-49`")
        );
        for bad in [
            "@cldr-49",
            "DE@cldr-49",
            "german@cldr-49",
            "de-u@cldr-49",
            "de-u-ks@cldr-49",
            "de-u-ks-level9@cldr-49",
            "de-u-ks-level1-ks-level2@cldr-49",
            "de-u-co-phonebk@cldr-49",
            "de-x-private@cldr-49",
            "de-hant@cldr-49",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
