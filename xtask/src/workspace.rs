use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{listing, step};

/// The cargo workspace `cargo xtask` runs in, as `cargo metadata` reports it.
#[derive(Debug)]
pub struct Workspace {
    pub root: PathBuf,
    pub target: PathBuf,
}

impl Workspace {
    pub fn find() -> Result<Self, String> {
        let mut metadata = Command::new("cargo");
        metadata.args(["metadata", "--format-version", "1", "--no-deps"]);
        let output = step::output("cargo metadata", &mut metadata)?;
        let value: serde_json::Value = serde_json::from_slice(&output)
            .map_err(|e| format!("cargo metadata: its output is no JSON: {e}"))?;
        let path = |field: &str| {
            value[field]
                .as_str()
                .map(PathBuf::from)
                .ok_or_else(|| format!("cargo metadata: no string `{field}` in its output"))
        };
        Ok(Self {
            root: path("workspace_root")?,
            target: path("target_directory")?,
        })
    }

    pub fn manifest(&self) -> PathBuf {
        self.root.join("Cargo.toml")
    }

    /// Returns `cargo SUBCOMMAND --manifest-path` of this workspace, building into its target
    /// directory.
    pub fn cargo(&self, subcommand: &[&str]) -> Command {
        let mut cargo = self.command("cargo");
        cargo
            .args(subcommand)
            .arg("--manifest-path")
            .arg(self.manifest());
        cargo
    }

    /// Returns a command of `program` that builds into this workspace's target directory.
    pub fn command(&self, program: impl AsRef<Path>) -> Command {
        let mut command = Command::new(program.as_ref());
        command
            .env("CARGO_TARGET_DIR", &self.target)
            .env(listing::UNDER_XTASK, "1");
        command
    }
}
