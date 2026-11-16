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

/// Run a workspace script as a gate: directly (a shim or the binary itself), then through a shell.
/// Windows cannot execute a .sh directly and its shells want the file NAME from the script's own
/// directory rather than a Windows path; the standard Git-bash locations are tried explicitly
/// because a plain test process on a Windows runner has no shell on PATH.
pub fn run_script(script: &Path, direct_cwd: &Path) -> std::io::Result<std::process::Output> {
    use std::process::Command;
    if let Ok(out) = Command::new(script).current_dir(direct_cwd).output() {
        return Ok(out);
    }
    let parent = script.parent().unwrap_or(Path::new(".")).to_path_buf();
    let name = script
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    // Git bash FIRST on Windows: the `bash` a Windows host resolves may be WSL's stub, which
    // fails with "no installed distributions" when there is no distro.
    const SHELLS: [&str; 5] = [
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
        "bash",
        "sh",
    ];
    let mut last: Option<std::io::Error> = None;
    for shell in SHELLS {
        match Command::new(shell).arg(&name).current_dir(&parent).output() {
            Ok(out) => return Ok(out),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no shell could run the gate")))
}
