//! Repo-side write auditing: the tier-0 store has its `audit` table, and every repo write gets a line here.
//!
//! `memory_save`, `promote --apply` and `landmine` change versioned files. The day line and git are the record of what
//! changed; this log is the trail of which tool action touched which files, and whether it was applied or rolled back.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub fn record(root: &Path, action: &str, outcome: &str, files: &[String]) -> std::io::Result<()> {
    let stamp = std::process::Command::new("date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let dir = root.join("memory/logs");
    std::fs::create_dir_all(&dir)?;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("repo-writes.log"))?;
    let touched = if files.is_empty() {
        "-".to_string()
    } else {
        files.join(" ")
    };
    writeln!(f, "{stamp} {action} {outcome} {touched}")
}
