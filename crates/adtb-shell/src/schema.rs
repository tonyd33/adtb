//! Turn a schema file of declarations into its first migration.

use adtb_lang::check::schema_migration;
use adtb_lang::parse::{Options, parse_migration, parse_schema};
use adtb_lang::span::Diag;

/// Returns the first migration `text` gives: `text` itself when it is a migration, or the
/// migration [`schema_migration`] replays a schema file as. Returns the errors of building and
/// checking a schema file that does not check, and the parse errors of whichever form `text`
/// reads further as when it is neither.
pub fn first_migration(text: &str) -> Result<String, String> {
    let migration = parse_migration(text, Options::default());
    if migration.diags.is_empty() {
        return Ok(text.to_owned());
    }
    let schema = parse_schema(text, Options::default());
    let diags = match schema.value {
        Some(file) if schema.diags.is_empty() => match schema_migration(text, &file, &[]) {
            Ok(m) => return Ok(m),
            Err(ds) => ds.into_iter().map(|d| d.diag).collect(),
        },
        _ if reach(&schema.diags) > reach(&migration.diags) => schema.diags,
        _ => migration.diags,
    };
    let rendered: Vec<String> = diags.iter().map(|d| d.render(text)).collect();
    Err(rendered.join("\n"))
}

fn reach(diags: &[Diag]) -> u32 {
    diags.iter().map(|d| d.span.start).min().unwrap_or(0)
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
    fn a_schema_file_whose_relation_and_fn_need_each_other_is_refused() {
        let s = "module demo
relation ps : P key id computed m: I64 = f()
type P = { id: I64, n: I64 }
fn f () -> I64 = count(1 for p in ps)
";
        assert_eq!(
            Err("2:1: the declaration needs itself through the declarations it names".to_owned()),
            first_migration(s)
        );
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
