use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::ast::{Decl, SchemaFile};
use adtb_lang::lex::{Tok, lex};
use adtb_lang::parse::{Options, parse_migration, parse_schema};
use adtb_lang::span::{Diag, Span};

/// Returns the first migration `text` gives: `text` itself when it is a migration, or the
/// migration a schema file replays as, each declaration after those it names and prefixed by
/// its verb. Returns the parse errors of whichever form `text` reads further as when it is
/// neither.
pub fn first_migration(text: &str) -> Result<String, String> {
    let migration = parse_migration(text, Options::default());
    if migration.diags.is_empty() {
        return Ok(text.to_owned());
    }
    let schema = parse_schema(text, Options::default());
    let diags = match schema.value {
        Some(file) if schema.diags.is_empty() => return Ok(replay(text, &file)),
        _ if reach(&schema.diags) > reach(&migration.diags) => schema.diags,
        _ => migration.diags,
    };
    let rendered: Vec<String> = diags.iter().map(|d| d.render(text)).collect();
    Err(rendered.join("\n"))
}

fn reach(diags: &[Diag]) -> u32 {
    diags.iter().map(|d| d.span.start).min().unwrap_or(0)
}

fn replay(text: &str, file: &SchemaFile) -> String {
    let module = &file.module.text;
    let mut out = format!("module {module}\nadd module {module}\n");
    for i in order(text, &file.decls) {
        let d = &file.decls[i];
        let verb = match d {
            Decl::Fn(_) | Decl::Aggregate(_) => "define ",
            Decl::Store(_) => "",
            _ => "add ",
        };
        out.push_str(verb);
        out.push_str(&text[span(d).range()]);
        out.push('\n');
    }
    out
}

/// Returns the indexes of `decls` in file order, except that each declaration follows every
/// declaration whose name it mentions. A type follows only the types it mentions, since a
/// reference may name a relation variable added later.
fn order(text: &str, decls: &[Decl]) -> Vec<usize> {
    let declared: BTreeMap<&str, usize> = decls
        .iter()
        .enumerate()
        .filter_map(|(i, d)| Some((name(d)?, i)))
        .collect();
    let needs: Vec<BTreeSet<usize>> = decls
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let src = &text[span(d).range()];
            lex(src)
                .0
                .iter()
                .filter(|t| matches!(t.tok, Tok::Lower | Tok::Upper))
                .filter_map(|t| declared.get(&src[t.span.range()]))
                .copied()
                .filter(|&j| j != i && (!is_type(d) || is_type(&decls[j])))
                .collect()
        })
        .collect();
    let mut placed = vec![false; decls.len()];
    let mut out = Vec::with_capacity(decls.len());
    for i in 0..decls.len() {
        place(i, &needs, &mut placed, &mut out);
    }
    out
}

/// Append `i` to `out` after what it needs. A cycle places its members in visiting order.
fn place(i: usize, needs: &[BTreeSet<usize>], placed: &mut [bool], out: &mut Vec<usize>) {
    if placed[i] {
        return;
    }
    placed[i] = true;
    for &j in &needs[i] {
        place(j, needs, placed, out);
    }
    out.push(i);
}

fn name(d: &Decl) -> Option<&str> {
    Some(match d {
        Decl::Type(t) => &t.name.text,
        Decl::Relation(r) => &r.name.name.text,
        Decl::Set(s) => &s.name.text,
        Decl::Fn(f) => &f.name.text,
        Decl::Aggregate(a) => &a.name.text,
        Decl::Constraint(_) | Decl::Check(_) | Decl::Store(_) => return None,
    })
}

fn is_type(d: &Decl) -> bool {
    matches!(d, Decl::Type(_))
}

fn span(d: &Decl) -> Span {
    match d {
        Decl::Type(t) => t.span,
        Decl::Relation(r) => r.span,
        Decl::Set(s) => s.span,
        Decl::Fn(f) => f.span,
        Decl::Constraint(c) => c.span,
        Decl::Check(c) => c.span,
        Decl::Store(s) => s.span,
        Decl::Aggregate(a) => a.span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_migration_is_kept_as_written() {
        let m = "module demo\nadd module demo\nadd type P = { id: Key ps }\n";
        assert_eq!(Ok(m.to_owned()), first_migration(m));
    }

    #[test]
    fn a_schema_file_replays_with_each_relation_after_its_type() {
        let s = "module demo
relation pets : Pet key name
type Pet { name: Text } = Cat { lives: I64 } | Dog { good: Bool }
fn lives (c: Pet.Cat) -> I64 = c.lives
";
        assert_eq!(
            Ok("module demo
add module demo
add type Pet { name: Text } = Cat { lives: I64 } | Dog { good: Bool }
add relation pets : Pet key name
define fn lives (c: Pet.Cat) -> I64 = c.lives
"
            .to_owned()),
            first_migration(s)
        );
    }

    #[test]
    fn a_type_naming_a_later_relation_stays_first() {
        let s = "module world
type Region { id: Key regions, parent: Option (Key regions.Settled) } = Wild | Town
relation regions : Region key id generate sequence
set Settled of regions = Town
";
        let m = first_migration(s).expect("a schema file");
        let lines: Vec<&str> = m
            .lines()
            .map(|l| l.split(' ').nth(1).unwrap_or(""))
            .collect();
        assert_eq!(vec!["world", "module", "type", "relation", "set"], lines);
    }

    #[test]
    fn a_schema_like_text_of_neither_form_reports_the_schema_form_errors() {
        assert_eq!(
            Err("2:24: expected a field name, found `}`".to_owned()),
            first_migration("module demo\ntype P = { id: Key ps, }}\n")
        );
    }

    #[test]
    fn a_migration_like_text_of_neither_form_reports_the_migration_form_errors() {
        let text =
            "module demo\nadd module demo\nadd type P = { id: Key ps }\nadd relation ps P key id\n";
        assert_eq!(
            Err("4:17: expected `:`, found `P`".to_owned()),
            first_migration(text)
        );
    }
}
