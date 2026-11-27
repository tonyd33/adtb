use std::path::Path;
use std::process::{Command, Stdio};

use super::{Tree, problems};

fn write(root: &Path, file: &str, text: &str) {
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().expect("a file under a directory"))
        .expect("the scratch checkout is written");
    std::fs::write(&path, text).expect("the scratch checkout is written");
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git starts");
    assert!(status.success(), "git {args:?}: {status}");
}

#[test]
fn only_tracked_files_are_read_or_held() {
    let root = std::env::temp_dir().join(format!("xtask-repo-{}", std::process::id()));
    let tracked = [
        (".gitignore", "/work/\n"),
        (".config/nextest.toml", "[profile.default]\n"),
        ("docs/SUMMARY.md", "- [A](manual/a.md)\n"),
        ("docs/manual/a.md", "# A\n"),
        ("docs/design/c.md", "# C\n"),
        ("lint/CLAUDE.md", "@AGENTS.md\n"),
        (
            "lint/AGENTS.md",
            "# Lint\n\n`rules.toml` holds `held_word`, not `notes_word`; see `work/AGENTS.md`.\n",
        ),
        ("lint/rules.toml", "held_word = 1\n"),
    ];
    let untracked = [
        ("work/AGENTS.md", "# Work **bold**\n\n`unheld_word`\n"),
        ("scratch/AGENTS.md", "`unheld_word`\n"),
        ("docs/manual/b.md", "# B\n"),
        ("lint/notes.md", "notes_word\n"),
    ];
    for (file, text) in tracked.into_iter().chain(untracked) {
        write(&root, file, text);
    }
    git(&root, &["init", "-q"]);
    for (file, _) in tracked {
        git(&root, &["add", "-f", file]);
    }
    let found = Tree::tracked(&root).and_then(|tree| problems(&tree));
    std::fs::remove_dir_all(&root).expect("the scratch checkout is removed");
    assert_eq!(
        found,
        Ok(vec![
            "lint/AGENTS.md: names `work/AGENTS.md`, which the tree does not hold".to_owned(),
            "lint/AGENTS.md: names `notes_word`, which no file under its directory or a path it names holds".to_owned(),
            "docs/SUMMARY.md omits [\"design/c.md\"]".to_owned(),
        ])
    );
}
