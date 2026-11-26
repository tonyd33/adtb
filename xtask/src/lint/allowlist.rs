use std::collections::BTreeMap;

/// Per row, per file, a count of sites.
pub type Counts = BTreeMap<String, BTreeMap<String, u64>>;

pub const PATH: &str = "lint/allowlist.toml";

const HEADER: &str = "\
# The sites each ratchet row of lint/rules.toml allows, per file. `cargo xtask lint` fails when a
# file has more sites than its entry allows (none without an entry), and when it has fewer: lower
# the entry to the new count and delete it at 0. Counts only fall: `cargo xtask lint --against REV`
# fails when an entry is above the one the allowlist at REV holds, so a rise is a maintainer's act
# on main. `cargo xtask lint --counts` prints this file for the tree.
";

/// Returns `counts` as an allowlist file, entries of 0 left out.
pub fn render(counts: &Counts) -> String {
    let mut text = HEADER.to_owned();
    for (row, files) in counts {
        if files.values().all(|&count| count == 0) {
            continue;
        }
        text.push_str(&format!("\n[{row}]\n"));
        for (file, count) in files.iter().filter(|&(_, &count)| count > 0) {
            text.push_str(&format!("{file:?} = {count}\n"));
        }
    }
    text
}

/// Read an allowlist: a table per row of `"FILE" = COUNT` entries, each count above 0.
pub fn parse(text: &str) -> Result<Counts, String> {
    let counts: Counts = toml::from_str(text).map_err(|e| e.to_string())?;
    for (row, files) in &counts {
        if let Some((file, _)) = files.iter().find(|&(_, &count)| count == 0) {
            return Err(format!("[{row}] lists {file} at 0: delete the entry"));
        }
    }
    Ok(counts)
}
