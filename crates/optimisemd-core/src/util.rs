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
    if let Err(e) = fs::rename(&tmp, path) {
        // Do not leave the temp behind when the rename is what failed.
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // A rename is atomic, but durability needs the directory flushed: without this a power cut can
    // lose the newest write (giving back the old file silently). Windows cannot open a directory
    // for flushing, so there the guarantee stays "atomic, not durable" (v0.4.9).
    #[cfg(unix)]
    {
        if let Ok(d) = fs::File::open(&dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

/// Whether this host can run a workspace shell script at all. The shell-dependent tests ask this
/// instead of being compiled out, so a Windows CI box with Git bash runs them and a bare one says
/// so out loud (REQUIRE_SHELL=1 makes the absence a failure).
pub fn shell_available() -> bool {
    shell_can_run_scripts(&std::env::temp_dir())
}

/// Can any shell actually run a script that lives in `dir`? Proves it the only way that counts: write
/// a tiny script there, run it the same way the gate is run (a relative name, the directory as the
/// working directory), and read back what it wrote.
///
/// A Windows host whose `bash` is the WSL bridge answers 0 having run nothing (and its `printf` works
/// while a script in a Windows temp directory silently does not), so the loose probe let a save pass
/// without running its checker (2026-11-17). Git for Windows runs scripts where they live; the WSL
/// bridge does not, and is therefore not a usable gate shell.
pub fn shell_can_run_scripts(dir: &Path) -> bool {
    shells_for_gate()
        .into_iter()
        .any(|s| shell_runs_a_script(s, dir))
}

/// The shell candidates, in order: Git for Windows first (a WSL `bash` on `PATH` must never win),
/// then the usual unix names.
fn shells_for_gate() -> Vec<&'static str> {
    vec![
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
        "bash",
        "sh",
    ]
}

fn shell_runs_a_script(shell: &str, dir: &Path) -> bool {
    use std::process::Command;
    let tag = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let name = format!(".optimisemd-probe-{tag}.sh");
    let out = dir.join(format!(".optimisemd-probe-{tag}.out"));
    let script = dir.join(&name);
    let _ = fs::remove_file(&out);
    if fs::write(
        &script,
        format!(
            "printf ok > '{}'\n",
            out.file_name().unwrap().to_string_lossy()
        ),
    )
    .is_err()
    {
        return false;
    }
    let ran = matches!(
        Command::new(shell).arg(&name).current_dir(dir).output(),
        Ok(o) if o.status.success()
    );
    let wrote = fs::read_to_string(&out)
        .map(|t| t.trim() == "ok")
        .unwrap_or(false);
    let _ = fs::remove_file(&script);
    let _ = fs::remove_file(&out);
    ran && wrote
}

/// Kept for the unix test that proves a silent shell is rejected (Windows has no equivalent stub).
#[cfg(all(test, unix))]
fn shell_works(shell: &str) -> bool {
    shell_runs_a_script(shell, &std::env::temp_dir())
}

/// Keep the newest `keep` backup folders in `dir` whose names start with one of the prefixes and
/// remove older ones. Every save and every apply leaves one; nothing else prunes them (v0.4.9).
///
/// Age comes from the newest FILE inside each backup, never the directory's own mtime: Windows
/// refreshes that when a child is created, so back-dated content still left the directory "now" and
/// the wrong backups were pruned (caught on the Windows host, 2026-11-17).
pub fn prune_backups(dir: &Path, prefixes: &[&str], keep: usize) -> usize {
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    let mut found: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    for e in rd.flatten() {
        let path = e.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if !path.is_dir() || !prefixes.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let mut newest: Option<std::time::SystemTime> = None;
        if let Ok(inner) = fs::read_dir(&path) {
            for child in inner.flatten() {
                if let Ok(t) = child.metadata().and_then(|m| m.modified()) {
                    newest = Some(newest.map_or(t, |w| w.max(t)));
                }
            }
        }
        let when = newest
            .or_else(|| e.metadata().and_then(|m| m.modified()).ok())
            .unwrap_or(std::time::UNIX_EPOCH);
        found.push((when, path));
    }
    // Oldest first, then delete everything past the newest `keep`.
    found.sort_by_key(|(t, _)| *t);
    let drop_count = found.len().saturating_sub(keep);
    found
        .drain(0..drop_count)
        .map(|(_, p)| {
            let _ = fs::remove_dir_all(&p);
            1
        })
        .sum()
}

/// Remove temp files a crashed write left behind (`.<name>.tmp-<pid>`), older than a day so a
/// concurrent writer's live temp is never touched.
pub fn prune_tmp(dir: &Path, older_than: std::time::Duration) -> usize {
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    rd.flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with('.') && name.contains(".tmp-")
        })
        .filter(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .map(|age| age >= older_than)
                .unwrap_or(false)
        })
        .map(|e| {
            let _ = fs::remove_file(e.path());
            1
        })
        .sum()
}

/// Put every file in `backup` back at its relative path under `root`. Best effort: the first error
/// is returned after the rest are attempted, so one unreadable copy does not abandon the others.
pub fn restore_backup(root: &Path, backup: &Path, rels: &[String]) -> std::io::Result<()> {
    let mut first: Option<std::io::Error> = None;
    for rel in rels {
        let src = backup.join(rel);
        if !src.exists() {
            continue;
        }
        let dst = root.join(rel);
        if let Some(d) = dst.parent() {
            fs::create_dir_all(d)?;
        }
        if let Err(e) = fs::copy(&src, &dst) {
            first = first.take().or(Some(e));
        }
    }
    match first {
        Some(e) => Err(e),
        None => Ok(()),
    }
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
    run_script_with_args(script, direct_cwd, &[])
}

/// Run a workspace script with arguments, capturing its output.
pub fn run_script_with_args(
    script: &Path,
    direct_cwd: &Path,
    args: &[&str],
) -> std::io::Result<std::process::Output> {
    use std::process::Command;
    if let Ok(out) = Command::new(script)
        .args(args)
        .current_dir(direct_cwd)
        .output()
    {
        return Ok(out);
    }
    let (shell, name, parent) = shell_for(script)?;
    Command::new(shell)
        .arg(&name)
        .args(args)
        .current_dir(&parent)
        .output()
}

/// Run a workspace script with arguments, letting its own output go straight through (a finder's
/// answers must reach the user, not be captured).
pub fn run_script_status(
    script: &Path,
    direct_cwd: &Path,
    args: &[&str],
) -> std::io::Result<std::process::ExitStatus> {
    use std::process::Command;
    if let Ok(st) = Command::new(script)
        .args(args)
        .current_dir(direct_cwd)
        .status()
    {
        return Ok(st);
    }
    let (shell, name, parent) = shell_for(script)?;
    Command::new(shell)
        .arg(&name)
        .args(args)
        .current_dir(&parent)
        .status()
}

/// The shell that may run `script`, the script's name and its directory. Only a shell that proves it
/// can run a script where the script lives counts: the WSL bridge answers 0 having done nothing,
/// which would let a red checker pass (2026-11-17).
fn shell_for(
    script: &Path,
) -> std::io::Result<(&'static str, std::ffi::OsString, std::path::PathBuf)> {
    let parent = script.parent().unwrap_or(Path::new(".")).to_path_buf();
    let name = script
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    match shells_for_gate()
        .into_iter()
        .find(|s| shell_runs_a_script(s, &parent))
    {
        Some(shell) => Ok((shell, name, parent)),
        None => Err(std::io::Error::other(
            "no usable shell to run the gate (on Windows install Git for Windows)",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("optimisemd-util-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_rewrite_replaces_the_content_and_leaves_no_temp_file() {
        let dir = temp("rewrite");
        let f = dir.join("MEMORY.md");
        write_atomic(&f, "first\n").unwrap();
        write_atomic(&f, "second\n").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "second\n");
        let left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            left.iter().all(|n| !n.contains(".tmp-")),
            "temp left behind: {left:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    // A write that cannot even create its temp must leave the previous content untouched.
    #[cfg(unix)]
    #[test]
    fn a_failed_write_leaves_the_previous_content() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("fail");
        let f = dir.join("MEMORY.md");
        write_atomic(&f, "old\n").unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
        let out = write_atomic(&f, "new\n");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            out.is_err(),
            "the write into a read-only directory must fail"
        );
        assert_eq!(fs::read_to_string(&f).unwrap(), "old\n");
        let _ = fs::remove_dir_all(&dir);
    }

    // The WSL stub on a Windows host exits 0 having run nothing: a shell like that must not count,
    // or a red checker would pass.
    #[cfg(unix)]
    #[test]
    fn a_silent_shell_does_not_count_as_a_shell() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("fakeshell");
        let fake = dir.join("bash");
        fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            !shell_works(fake.to_str().unwrap()),
            "a shell that prints nothing must be rejected"
        );
        assert!(shell_works("sh"), "a real shell must be accepted");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pruning_removes_only_stale_temp_files() {
        let dir = temp("tmp");
        let stale = dir.join(".MEMORY.md.tmp-1");
        let live = dir.join(".MEMORY.md.tmp-2");
        fs::write(&stale, "").unwrap();
        fs::write(&live, "").unwrap();
        let f = fs::File::create(&stale).unwrap();
        f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(90_000))
            .unwrap();
        drop(f);
        let removed = prune_tmp(&dir, std::time::Duration::from_secs(86_400));
        assert_eq!(removed, 1);
        assert!(!stale.exists());
        assert!(
            live.exists(),
            "a concurrent writer's live temp must survive"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
