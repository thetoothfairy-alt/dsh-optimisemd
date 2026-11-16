//! The hardening fixes, each through the built binary: the finder's query handling, the promotion dry-run's audit,
//! and the session reports agreeing on a long session.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn temp(label: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "optimisemd-hard-{}-{label}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::canonicalize(&dir).unwrap()
}

fn ws_key(root: &Path) -> String {
    format!(
        "--{}--",
        root.to_string_lossy()
            .trim_start_matches('/')
            .replace('/', "-")
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

// The finder stub is a bash script the binary executes directly: a unix contract.
#[cfg(unix)]
#[test]
fn find_does_not_hijack_a_question_that_starts_with_index() {
    let root = temp("find");
    let home = temp("find-home");
    for p in [
        "memory/logs/session-digest.md",
        "memory/logs/memento-export.md",
        "memory/logs/find-index.json",
    ] {
        fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
        fs::write(root.join(p), "stub\n").unwrap();
    }
    fs::create_dir_all(root.join("bin")).unwrap();
    let stub = root.join("bin/dsh-belfind");
    fs::write(
        &stub,
        "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > \"$(dirname \"$0\")/finder-args.txt\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let out = run(
        &root,
        &home,
        &["find", "index", "of", "the", "memory", "store"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let args = fs::read_to_string(root.join("bin/finder-args.txt")).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        vec!["find", "index", "of", "the", "memory", "store"],
        "a question beginning with 'index' must reach the finder unchanged"
    );

    // The bare `index` word is still the reindex shorthand.
    let out = run(&root, &home, &["find", "index"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let args = fs::read_to_string(root.join("bin/finder-args.txt")).unwrap();
    assert_eq!(args.lines().collect::<Vec<_>>(), vec!["index"]);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn promote_dry_run_writes_nothing_and_records_no_audit() {
    let root = temp("dryrun");
    let home = temp("dryrun-home");
    fs::create_dir_all(root.join("docs/memory-details")).unwrap();
    fs::create_dir_all(root.join("memory")).unwrap();
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::write(
        root.join("docs/memory-details/01.md"),
        "# Memory details — 01\n\n### §test-subject\n\nLong form.\n",
    )
    .unwrap();
    fs::write(
        root.join("memory/2026-11-13.md"),
        "# 2026-11-13\n\n- today's line\n",
    )
    .unwrap();
    fs::write(
        root.join("MEMORY.md"),
        "# MEMORY.md\n\n## Subject facts\n- **A quiet row.** (2020-01-01) — `docs/memory-details/01.md`\n",
    )
    .unwrap();
    let gate = root.join("scripts/check-memory-index.sh");
    fs::write(&gate, "#!/usr/bin/env bash\necho 'indexes 1 · fact rows 1 · detail rows 0 · findings 0'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&gate, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let before = fs::read_to_string(root.join("MEMORY.md")).unwrap();
    let out = run(&root, &home, &["promote", "--apply", "--dry-run"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read_to_string(root.join("MEMORY.md")).unwrap(), before);
    let log = root.join("memory/logs/repo-writes.log");
    if log.exists() {
        let text = fs::read_to_string(&log).unwrap();
        assert!(!text.contains("promote-apply"), "{text}");
    }
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn the_reports_skip_transcripts_from_another_workspace() {
    let root = temp("contam");
    let home = temp("contam-home");
    fs::create_dir_all(root.join("docs")).unwrap();
    // Two roots can map to the same session folder name; a transcript that records its workspace
    // must not leak into another workspace's reports.
    let mine = format!(
        "{{\"type\":\"session\",\"id\":\"session-aaaaaaaa\",\"createdAt\":1794796632985,\"cwd\":\"{}\"}}\n\
         {{\"type\":\"request/header\"}}\n\
         {{\"type\":\"session/title\",\"data\":{{\"title\":\"Mine session\"}}}}\n",
        root.display()
    );
    let theirs = "{\"type\":\"session\",\"id\":\"session-bbbbbbbb\",\"createdAt\":1794796632985,\"cwd\":\"/somewhere-else\"}\n\
         {\"type\":\"request/header\"}\n\
         {\"type\":\"session/title\",\"data\":{\"title\":\"Theirs session\"}}\n"
        .to_string();
    let base = home.join("sessions").join(ws_key(&root));
    for (name, body) in [("session-mine", mine), ("session-theirs", theirs)] {
        fs::create_dir_all(base.join(name)).unwrap();
        fs::write(base.join(name).join("session.v4.jsonl"), body).unwrap();
    }

    let out = run(&root, &home, &["session-index"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let index = fs::read_to_string(root.join("docs/session-index.md")).unwrap();
    assert!(index.contains("Mine session"), "{index}");
    assert!(!index.contains("Theirs session"), "{index}");

    let out = run(&root, &home, &["session-digest"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let digest = fs::read_to_string(root.join("memory/logs/session-digest.md")).unwrap();
    assert!(digest.contains("Mine session"), "{digest}");
    assert!(!digest.contains("Theirs session"), "{digest}");
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn the_reports_agree_on_a_session_whose_events_start_late() {
    let root = temp("parity");
    let home = temp("parity-home");
    fs::create_dir_all(root.join("docs")).unwrap();
    let mut body = String::new();
    // More than 200,000 characters of filler before the first recognised event.
    for i in 0..700 {
        body.push_str(&format!(
            "{{\"type\":\"filler\",\"text\":\"{i:04} {}\"}}\n",
            "x".repeat(320)
        ));
    }
    body.push_str("{\"createdAt\":1794796632985,\"type\":\"request/header\"}\n");
    body.push_str("{\"type\":\"session/title\",\"data\":{\"title\":\"Late title session\"}}\n");
    fs::create_dir_all(
        home.join("sessions")
            .join(ws_key(&root))
            .join("session-late"),
    )
    .unwrap();
    fs::write(
        home.join("sessions")
            .join(ws_key(&root))
            .join("session-late/session.v4.jsonl"),
        body,
    )
    .unwrap();

    let out = run(&root, &home, &["session-index"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let index = fs::read_to_string(root.join("docs/session-index.md")).unwrap();
    assert!(index.contains("Late title session"), "{index}");

    let out = run(&root, &home, &["session-digest"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let digest = fs::read_to_string(root.join("memory/logs/session-digest.md")).unwrap();
    assert!(digest.contains("Late title session"), "{digest}");
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&home);
}
