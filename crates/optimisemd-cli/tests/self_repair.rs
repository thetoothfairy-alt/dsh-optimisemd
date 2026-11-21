//! Self-repair, run as a session runs it: the built binary, a workspace and a fixture DSH_HOME.
//!
//! The 2026-11-21 review found four classes of maintenance fault: a stale generated report, a guard
//! that fails while nothing is wrong (the date-blind store check), a guard that cannot prove itself
//! (the truncated parity script), and discovery gaps. The report repairs are automatic; the guard
//! faults are reported with the reason, never silently passed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn no_shell(what: &str) {
    if std::env::var("REQUIRE_SHELL").as_deref() == Ok("1") {
        panic!("{what} needs a POSIX shell and this host has none (REQUIRE_SHELL=1)");
    }
    eprintln!("SKIPPED {what}: no POSIX shell on this host (REQUIRE_SHELL=1 makes that a failure)");
}

fn temp(label: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "optimisemd-self-{}-{label}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let abs = fs::canonicalize(&dir).unwrap();
    let s = abs.to_string_lossy().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest.to_string())
    } else {
        abs
    }
}

fn ws_key(root: &Path) -> String {
    format!(
        "--{}--",
        root.to_string_lossy()
            .trim_start_matches(['/', '\\'])
            .replace(['/', '\\', ':'], "-")
    )
}

fn run(root: &Path, home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dsh-optimisemd"))
        .args(args)
        .current_dir(root)
        .env("DSH_HOME", home)
        .output()
        .unwrap()
}

fn transcript(dir: &Path, body: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("session.v4.jsonl"), body).unwrap();
}

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_dated(root: &Path, args: &[&str], date: &str) {
    let out = Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn script(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn self_repair_rebuilds_a_missing_session_index_and_digest() {
    let root = temp("reports");
    fs::create_dir_all(root.join("docs")).unwrap();
    let home = temp("reports-home");
    transcript(
        &home
            .join("sessions")
            .join(ws_key(&root))
            .join("session-one"),
        "{\"createdAt\":1794796632985,\"type\":\"request/header\"}\n\
         {\"type\":\"session/title\",\"data\":{\"title\":\"A real session\"}}\n\
         {\"type\":\"user/message\",\"data\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"A question about the widget and the email rig\"}]}}\n",
    );
    let out = run(&root, &home, &["self-repair", "--fix"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "self-repair failed: {text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("[fix]"), "{text}");
    let index = fs::read_to_string(root.join("docs/session-index.md")).unwrap();
    assert!(index.contains("**1 sessions**"), "{index}");
    let digest = fs::read_to_string(root.join("memory/logs/session-digest.md")).unwrap();
    assert!(digest.contains("A real session"), "{digest}");
}

#[test]
fn self_repair_without_fix_writes_no_report() {
    let root = temp("reports-check");
    fs::create_dir_all(root.join("docs")).unwrap();
    let home = temp("reports-check-home");
    transcript(
        &home
            .join("sessions")
            .join(ws_key(&root))
            .join("session-one"),
        "{\"createdAt\":1794796632985,\"type\":\"request/header\"}\n\
         {\"type\":\"session/title\",\"data\":{\"title\":\"A real session\"}}\n",
    );
    let out = run(&root, &home, &["self-repair"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("[open]"));
    assert!(!root.join("docs/session-index.md").exists());
    assert!(!root.join("memory/logs/session-digest.md").exists());
}

#[test]
fn self_repair_rewrites_a_stale_measured_block() {
    let root = temp("counts");
    let home = temp("counts-home");
    fs::create_dir_all(root.join("docs/memory-details")).unwrap();
    fs::create_dir_all(root.join("docs/runbooks")).unwrap();
    fs::create_dir_all(root.join("memory")).unwrap();
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::write(
        root.join("MEMORY.md"),
        "## Subject facts\n- **A fact.** (2026-01-01) — `docs/memory-details/01-a.md`\n",
    )
    .unwrap();
    fs::write(root.join("AGENTS.md"), "one\ntwo\n").unwrap();
    fs::write(root.join("docs/memory-details/01-a.md"), "# part\n").unwrap();
    fs::write(root.join("docs/runbooks/r1.md"), "# runbook\n").unwrap();
    fs::write(root.join("memory/2026-01-01.md"), "# day\n").unwrap();
    fs::write(
        root.join("scripts/nightly-maintenance.sh"),
        "step \"a\" x\n",
    )
    .unwrap();
    fs::write(
        root.join("docs/memory-process.md"),
        "# doc\n\n<!-- measured:start -->\nan old line\n<!-- measured:end -->\n",
    )
    .unwrap();
    transcript(
        &home
            .join("sessions")
            .join(ws_key(&root))
            .join("session-one"),
        "{\"createdAt\":1794796632985,\"type\":\"request/header\"}\n\
         {\"type\":\"session/title\",\"data\":{\"title\":\"A real session\"}}\n",
    );
    let out = run(&root, &home, &["self-repair", "--fix"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "self-repair failed: {text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc = fs::read_to_string(root.join("docs/memory-process.md")).unwrap();
    assert!(doc.contains("Measured"), "{doc}");
    assert!(doc.contains("1 fact rows"), "{doc}");
}

#[test]
fn self_repair_reports_the_broken_guards_without_repairing_them_by_guess() {
    if !optimisemd_core::util::shell_available() {
        return no_shell("self-repair's guard checks");
    }
    let root = temp("guards");
    let home = temp("guards-home");
    fs::create_dir_all(root.join("docs")).unwrap();
    script(
        &root,
        "scripts/check-discovery-coverage.sh",
        "#!/bin/sh\nprintf 'RUNBOOK not in docs/workflows.md: unifi.md\\n'\nexit 1\n",
    );
    script(&root, "tools/wschk/tests/parity.sh", "#!/bin/sh\nfail=0\n");
    fs::create_dir_all(root.join("memory/nightly")).unwrap();
    fs::write(
        root.join("memory/nightly/2026-11-21.log"),
        "  FAIL  discovery coverage (exit 1)\n  FAIL  store separation (exit 1)\nresult: FAILED\n",
    )
    .unwrap();
    let out = run(&root, &home, &["self-repair"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("discovery"), "{text}");
    assert!(text.contains("parity"), "{text}");
    assert!(text.contains("nightly"), "{text}");
}

#[test]
fn a_same_day_store_deletion_is_reported_as_a_guard_defect() {
    if !optimisemd_core::util::shell_available() {
        return no_shell("self-repair's store-separation check");
    }
    let root = temp("storesep");
    let home = temp("storesep-home");
    fs::create_dir_all(root.join("memory")).unwrap();
    git(&root, &["init", "-q"]);
    fs::write(root.join("memory/2026-01-01.md"), "one\ntwo\n").unwrap();
    git_dated(
        &root,
        &["add", "memory/2026-01-01.md"],
        "2026-01-01T09:00:00",
    );
    git_dated(
        &root,
        &["commit", "-q", "-m", "write the day"],
        "2026-01-01T09:00:00",
    );
    fs::write(root.join("memory/2026-01-01.md"), "one\n").unwrap();
    git_dated(
        &root,
        &["add", "memory/2026-01-01.md"],
        "2026-01-01T23:00:00",
    );
    git_dated(
        &root,
        &["commit", "-q", "-m", "same-day edit"],
        "2026-01-01T23:00:00",
    );
    script(
        &root,
        "scripts/check-store-separation.sh",
        "#!/bin/sh\nprintf '[2] memory/2026-01-01.md: its last commit removed 1 line(s) — the event log is append-only\\n'\nprintf 'stores: beliefs 0 · event dirs 1 · findings 1\\n'\nexit 1\n",
    );
    let out = run(&root, &home, &["self-repair"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("within the 7-day window"), "{text}");
    assert!(text.contains("date-blind"), "{text}");
}

#[test]
fn self_repair_rebuilds_a_stale_finder_index() {
    if !optimisemd_core::util::shell_available() {
        return no_shell("self-repair's finder rebuild");
    }
    let root = temp("finder");
    let home = temp("finder-home");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join("memory/logs")).unwrap();
    fs::write(
        root.join("memory/logs/session-digest.md"),
        "# digest\n- **a row**\n",
    )
    .unwrap();
    script(
        &root,
        "bin/dsh-belfind",
        "#!/bin/sh\nroot=\"$(cd \"$(dirname \"$0\")/..\" && pwd)\"\nmkdir -p \"$root/memory/logs\"\n: > \"$root/memory/logs/find-index.json\"\n: > \"$root/bin/index-ran\"\n",
    );
    let out = run(&root, &home, &["self-repair", "--fix"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "self-repair failed: {text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("[fix]") && text.contains("finder: rebuilt"),
        "{text}"
    );
    assert!(root.join("bin/index-ran").exists(), "{text}");
    assert!(root.join("memory/logs/find-index.json").exists(), "{text}");
}
