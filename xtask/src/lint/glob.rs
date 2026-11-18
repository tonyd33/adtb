use std::collections::BTreeSet;
use std::path::Path;

/// Returns whether `path`, a `/`-separated path relative to the root, matches `pattern`. A
/// pattern component of two stars matches any number of path components, and one star inside a
/// component matches any run of characters within it.
pub fn matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    components(&pattern, &path)
}

fn components(pattern: &[&str], path: &[&str]) -> bool {
    match (pattern.split_first(), path.split_first()) {
        (None, None) => true,
        (Some((&"**", rest)), _) => {
            components(rest, path)
                || path
                    .split_first()
                    .is_some_and(|(_, tail)| components(pattern, tail))
        }
        (Some((wanted, rest)), Some((name, tail))) => {
            component(wanted, name) && components(rest, tail)
        }
        _ => false,
    }
}

fn component(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((head, tail)) => name.strip_prefix(head).is_some_and(|rest| {
            (0..=rest.len())
                .filter(|&i| rest.is_char_boundary(i))
                .any(|i| component(tail, &rest[i..]))
        }),
    }
}

/// Returns the files under `root` that match an `include` pattern and no `exclude` pattern, as
/// `/`-separated paths relative to `root`. Directories named `target` and hidden directories are
/// not entered.
pub fn files(
    root: &Path,
    include: &[String],
    exclude: &[String],
) -> Result<BTreeSet<String>, String> {
    let mut found = BTreeSet::new();
    for pattern in include {
        let fixed: Vec<&str> = pattern
            .split('/')
            .take_while(|component| !component.contains('*'))
            .collect();
        let start = fixed.join("/");
        let mut candidates = Vec::new();
        walk(root, &start, &mut candidates)?;
        candidates.retain(|path| matches(pattern, path));
        if candidates.is_empty() {
            return Err(format!("the pattern {pattern:?} matches no file"));
        }
        found.extend(
            candidates
                .into_iter()
                .filter(|path| !exclude.iter().any(|excluded| matches(excluded, path))),
        );
    }
    Ok(found)
}

fn walk(root: &Path, relative: &str, found: &mut Vec<String>) -> Result<(), String> {
    let path = root.join(relative);
    let kind = std::fs::metadata(&path)
        .map_err(|e| format!("reading {}: {e}", path.display()))?
        .file_type();
    if !kind.is_dir() {
        found.push(relative.to_owned());
        return Ok(());
    }
    let entries =
        std::fs::read_dir(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("reading {}: {e}", path.display()))?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| format!("{} holds a name that is not UTF-8", path.display()))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("reading {}/{name}: {e}", path.display()))?;
        if kind.is_dir() && (name == "target" || name.starts_with('.')) {
            continue;
        }
        let child = if relative.is_empty() {
            name.to_owned()
        } else {
            format!("{relative}/{name}")
        };
        if kind.is_dir() {
            walk(root, &child, found)?;
        } else {
            found.push(child);
        }
    }
    Ok(())
}
