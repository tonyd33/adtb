/// Returns `name` in upper camel case: each part between underscores capitalised.
pub(crate) fn camel(name: &str) -> String {
    name.split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut cs = p.chars();
            cs.next()
                .map(|c| c.to_uppercase().chain(cs).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}

/// Returns `name` as a Rust identifier: raw when it is a keyword.
pub(crate) fn ident(name: &str) -> String {
    match name {
        "self" | "super" | "crate" | "Self" | "_" => format!("{name}_"),
        n if KEYWORDS.contains(&n) => format!("r#{n}"),
        n => n.to_owned(),
    }
}

/// Returns whether rustc's `non_camel_case_types` lint fires on a type named `name`.
pub(crate) fn lower_type_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_lowercase()) || name.contains('_')
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern", "false",
    "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "static", "struct", "trait", "true", "type", "unsafe", "use", "where",
    "while", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "try",
    "typeof", "unsized", "virtual", "yield",
];
