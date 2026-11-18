//! Write the rows a case's directives got in place of the rows they expect.

use std::fs;

use crate::{Block, Difference};

use super::{Case, Format, shown};

fn blocks(format: Format, text: &str) -> Result<Vec<Block>, Difference> {
    match format {
        Format::Conformance => crate::conformance_blocks(text),
        #[cfg(feature = "scripts")]
        Format::Scripts => crate::scripted::blocks(text),
        #[cfg(not(feature = "scripts"))]
        Format::Scripts => Err(Difference {
            line: 0,
            message: "the runner is built without the `scripts` feature".into(),
            got: None,
        }),
    }
}

/// Write `rows`, each the rows of the directive on a line of `case`, in place of the rows after
/// that directive's `----`.
///
/// Returns why nothing is written: a line holds no directive with rows, or the file does not
/// read back with the rows written.
pub(super) fn write(case: &Case, rows: &[(usize, Vec<String>)]) -> Result<(), String> {
    let at = shown(&case.root, &case.path);
    let text = fs::read_to_string(&case.path).map_err(|e| format!("{at}: {e}"))?;
    let unread = |d: Difference| format!("{at}:{}: {}", d.line, d.message);
    let found = blocks(case.format, &text).map_err(unread)?;
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut spans = Vec::new();
    for (line, got) in rows {
        let k = found
            .iter()
            .position(|b| b.line == *line)
            .ok_or_else(|| format!("{at}:{line}: no directive starts here"))?;
        let dashes = found[k]
            .dashes
            .ok_or_else(|| format!("{at}:{line}: the directive takes no rows"))?;
        spans.push((k, dashes, found[k].rows.len(), got));
    }
    spans.sort_by_key(|(_, dashes, _, _)| std::cmp::Reverse(*dashes));
    for (_, dashes, before, got) in &spans {
        lines.splice(*dashes..dashes + before, got.iter().cloned());
    }
    let mut written = lines.join("\n");
    if text.ends_with('\n') {
        written.push('\n');
    }
    let reread = blocks(case.format, &written).map_err(unread)?;
    for (k, _, _, got) in &spans {
        if reread.len() != found.len() || reread[*k].rows != **got {
            let line = found[*k].line;
            return Err(format!(
                "{at}:{line}: the rows it got do not read back as its rows"
            ));
        }
    }
    fs::write(&case.path, written).map_err(|e| format!("{at}: {e}"))
}
