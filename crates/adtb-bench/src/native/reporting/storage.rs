use super::*;
use std::os::unix::fs::MetadataExt;

pub(super) fn inventory(root: &Path) -> Result<Vec<Value>, String> {
    let mut pending = vec![root.to_owned()];
    let mut rows = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err("database inventory requires regular files".into());
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("database inventory requires UTF-8 paths")?
                .to_owned();
            if [
                "ledger.txt",
                "application.json",
                ".application.json.tmp",
                "calls.ndjson",
                "attempts.ndjson",
                "workflows.ndjson",
            ]
            .contains(&relative.as_str())
            {
                continue;
            }
            let allocated = metadata
                .blocks()
                .checked_mul(512)
                .ok_or("allocated byte count overflow")?;
            rows.push(
                json!({"path":relative,"logical_bytes":metadata.len(),"allocated_bytes":allocated}),
            );
        }
    }
    rows.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(rows)
}

pub(super) fn process_io() -> Value {
    match read_io() {
        Ok(counters) => json!({"status":"available","source":"/proc/self/io",
            "boundary":"after-final-close-before-artifact-write","counters":counters}),
        Err(reason) => json!({"status":"unavailable","reason":reason}),
    }
}

fn read_io() -> Result<BTreeMap<String, u64>, String> {
    let text =
        std::fs::read_to_string("/proc/self/io").map_err(|e| format!("/proc/self/io: {e}"))?;
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line.split_once(':').ok_or("malformed process I/O record")?;
        let value = value.trim().parse::<u64>().map_err(|e| e.to_string())?;
        if fields.insert(key.to_owned(), value).is_some() {
            return Err("duplicate process I/O counter".into());
        }
    }
    let names = [
        "rchar",
        "wchar",
        "syscr",
        "syscw",
        "read_bytes",
        "write_bytes",
        "cancelled_write_bytes",
    ];
    if names.iter().any(|name| !fields.contains_key(*name)) {
        return Err("missing process I/O counter".into());
    }
    fields.retain(|key, _| names.contains(&key.as_str()));
    Ok(fields)
}
