//! Shared plumbing: crash-safe file writes and local timestamps computed in Rust.
//!
//! Before v0.4.8 every writer truncated its target in place and every timestamp asked the system
//! `date` command — a crash could corrupt `MEMORY.md`, and the Windows build had no `date` to ask.
//! Both are now decided here.

use std::fs;
use std::io::Write;
use std::path::Path;

/// Write a file the crash-safe way: a temp file beside the target, flushed to disk, then renamed
/// over it. A crash or a full disk leaves either the old file or the new one — never a truncated one.
pub fn write_atomic(path: &Path, body: &str) -> std::io::Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    fs::create_dir_all(&dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Local today, YYYY-MM-DD — computed here, not by shelling out to `date`.
pub fn today_local() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Local "now", YYYY-MM-DD HH:MM — the reports' stamp.
pub fn local_now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

/// Local "now" with seconds — the audit trail's stamp.
pub fn local_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// An epoch in seconds as a local YYYY-MM-DD HH:MM.
pub fn local_datetime(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(local_now)
}
