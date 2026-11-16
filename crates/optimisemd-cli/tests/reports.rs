//! The four report commands, run as a session runs them: the built binary, a workspace, and a fixture DSH_HOME.
//!
//! These exist because the report commands had no test at all, and the 0.4.0 port emptied the session index for three
//! nights while every helper stayed green. Each test below goes through `main` — including the root canonicalisation —
//! so deleting that call fails a test instead of silently emptying the index.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// The finder and the wrapper are shell scripts: a host with no POSIX shell cannot run them. Say so
/// out loud instead of compiling the test out — and fail when REQUIRE_SHELL=1 (CI sets it).
fn no_shell(what: &str) {
    if std::env::var("REQUIRE_SHELL").as_deref() == Ok("1") {
        panic!("{what} needs a POSIX shell and this host has none (REQUIRE_SHELL=1)");
    }
    eprintln!("SKIPPED {what}: no POSIX shell on this host (REQUIRE_SHELL=1 makes that a failure)");
}

fn temp(label: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("optimisemd-it-{}-{label}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // The binary canonicalises its root and drops Windows' \\?\ prefix; the fixture must spell the
    // workspace the same way.
    let abs = fs::canonicalize(&dir).unwrap();
    let s = abs.to_string_lossy().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest.to_string())
    } else {
        abs
    }
}

fn ws_key(root: &Path) -> String {
    // Same normalisation as the binary: Windows roots carry : and \, which are illegal in a folder name.
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

#[test]
fn session_index_finds_this_workspace_from_the_default_root() {
    let root = temp("index");
    fs::create_dir_all(root.join("docs")).unwrap();
    let home = temp("index-home");
    transcript(
        &home
            .join("sessions")
            .join(ws_key(&root))
            .join("session-abc"),
        "{\"createdAt\":1794796632985,\"type\":\"request/header\"}\n\
         {\"type\":\"session/title\",\"data\":{\"title\":\"The provider title\"}}\n",
    );
    let out = run(&root, &home, &["session-index"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = fs::read_to_string(root.join("docs/session-index.md")).unwrap();
    assert!(report.contains("The provider title"), "{report}");
    assert!(report.contains("**1 sessions**"), "{report}");
}

#[test]
fn session_index_refuses_to_write_an_empty_index() {
    let root = temp("index-empty");
    fs::create_dir_all(root.join("docs")).unwrap();
    let home = temp("index-empty-home");
    let out = run(&root, &home, &["session-index"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no sessions"));
    assert!(!root.join("docs/session-index.md").exists());
}

#[test]
fn digest_is_workspace_scoped_and_reads_the_archive() {
    let root = temp("digest");
    let home = temp("digest-home");
    let ws = ws_key(&root);
    let live = "{\"type\":\"session/title\",\"data\":{\"title\":\"Live title\"}}\n\
                {\"type\":\"user/message\",\"data\":{\"content\":[{\"type\":\"text\",\
                \"text\":\"How do I send a file in an email with the rig, plainly\"}]}}\n";
    transcript(&home.join("sessions").join(&ws).join("session-live"), live);
    transcript(
        &home.join("sessions-archive").join(&ws).join("session-arch"),
        "{\"type\":\"session/title\",\"data\":{\"title\":\"Archived title\"}}\n\
         {\"type\":\"user/message\",\"data\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"An archived question about the drawing redraw\"}]}}\n",
    );
    transcript(
        &home
            .join("sessions")
            .join("--other--")
            .join("session-other"),
        "{\"type\":\"session/title\",\"data\":{\"title\":\"Other workspace\"}}\n",
    );
    let out = run(&root, &home, &["session-digest"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let digest = fs::read_to_string(root.join("memory/logs/session-digest.md")).unwrap();
    assert!(digest.contains("Live title"), "{digest}");
    assert!(digest.contains("Archived title"), "{digest}");
    assert!(!digest.contains("Other workspace"), "{digest}");
}

#[test]
fn digest_cuts_the_question_before_the_runtime_reminder() {
    let root = temp("digest-noise");
    let home = temp("digest-noise-home");
    let ws = ws_key(&root);
    transcript(
        &home.join("sessions").join(&ws).join("session-noise"),
        "{\"type\":\"user/message\",\"data\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"Do the thing with the widget\\n\\nTime sampled while preparing turn 1, step 1: 2026-11-16\"}]}}\n",
    );
    let out = run(&root, &home, &["session-digest"]);
    assert!(out.status.success());
    let digest = fs::read_to_string(root.join("memory/logs/session-digest.md")).unwrap();
    assert!(digest.contains("Do the thing with the widget"), "{digest}");
    assert!(!digest.contains("Time sampled"), "{digest}");
}

#[test]
fn digest_skips_harness_notice_messages() {
    let root = temp("digest-notice");
    let home = temp("digest-notice-home");
    let ws = ws_key(&root);
    transcript(
        &home.join("sessions").join(&ws).join("session-notice"),
        "{\"type\":\"session/title\",\"data\":{\"title\":\"A real session\"}}\n\
         {\"type\":\"user/message\",\"data\":{\"source\":{\"kind\":\"dsh-doublecheck\",\"form\":\"notice\"},\
         \"content\":[{\"type\":\"text\",\"text\":\"Red/green discipline: no failing test is on record\"}]}}\n\
         {\"type\":\"user/message\",\"data\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"A real question about the widget and the email rig\"}]}}\n",
    );
    let out = run(&root, &home, &["session-digest"]);
    assert!(out.status.success());
    let digest = fs::read_to_string(root.join("memory/logs/session-digest.md")).unwrap();
    assert!(digest.contains("A real question"), "{digest}");
    assert!(!digest.contains("Red/green discipline"), "{digest}");
}

#[test]
fn session_index_skips_a_malformed_transcript_beside_a_good_one() {
    let root = temp("index-malformed");
    fs::create_dir_all(root.join("docs")).unwrap();
    let home = temp("index-malformed-home");
    let ws = ws_key(&root);
    transcript(
        &home.join("sessions").join(&ws).join("session-bad"),
        "this is not a transcript at all\n",
    );
    transcript(
        &home.join("sessions").join(&ws).join("session-good"),
        "{\"createdAt\":1794796632985,\"type\":\"request/header\"}\n\
         {\"type\":\"session/title\",\"data\":{\"title\":\"The good title\"}}\n",
    );
    let out = run(&root, &home, &["session-index"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = fs::read_to_string(root.join("docs/session-index.md")).unwrap();
    assert!(report.contains("**1 sessions**"), "{report}");
    assert!(report.contains("The good title"), "{report}");
}

#[test]
fn digest_includes_a_session_with_no_question_or_title() {
    let root = temp("digest-notitle");
    let home = temp("digest-notitle-home");
    let ws = ws_key(&root);
    transcript(
        &home
            .join("sessions")
            .join(&ws)
            .join("2222bbbb-3333-4444-5555-666666666666"),
        "{\"createdAt\":1794796632985,\"type\":\"request/header\"}\n",
    );
    let out = run(&root, &home, &["session-digest"]);
    assert!(out.status.success());
    let digest = fs::read_to_string(root.join("memory/logs/session-digest.md")).unwrap();
    assert!(digest.contains("2222bbbb"), "{digest}");
}

#[test]
fn digest_skips_a_plugin_notice() {
    let root = temp("digest-plugin");
    let home = temp("digest-plugin-home");
    let ws = ws_key(&root);
    transcript(
        &home.join("sessions").join(&ws).join("session-plugin"),
        "{\"type\":\"session/title\",\"data\":{\"title\":\"A real session\"}}\n\
         {\"type\":\"user/message\",\"data\":{\"source\":{\"kind\":\"plugin\"},\
         \"content\":[{\"type\":\"text\",\"text\":\"The approval policy changed from \\\"ask\\\" to \\\"never\\\"\"}]}}\n\
         {\"type\":\"user/message\",\"data\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"A real question about the approval policy and the rig\"}]}}\n",
    );
    let out = run(&root, &home, &["session-digest"]);
    assert!(out.status.success());
    let digest = fs::read_to_string(root.join("memory/logs/session-digest.md")).unwrap();
    assert!(
        digest.contains("A real question about the approval"),
        "{digest}"
    );
    assert!(!digest.contains("The approval policy changed"), "{digest}");
}

#[test]
fn slim_measures_characters_in_a_fixture_workspace() {
    let root = temp("slim");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::write(
        root.join("scripts/check-context-files.sh"),
        "#!/usr/bin/env bash\nBUDGETS_BLOCK=$(cat <<'BUDGETS'\n('AGENTS.md', 150, 20000)\nBUDGETS\n)\n",
    )
    .unwrap();
    git(&root, &["init", "-q"]);
    fs::write(root.join("AGENTS.md"), "a—b\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    fs::write(root.join("AGENTS.md"), "a—bc\n").unwrap();
    let home = temp("slim-home");
    let out = run(&root, &home, &["slim"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cut in place"), "{stdout}");
    let report = fs::read_to_string(root.join("docs/md-slimming-report.md")).unwrap();
    assert!(report.contains("AGENTS.md"), "{report}");
    assert!(report.contains("cut in place"), "{report}");
}

#[test]
fn digest_reads_a_spliced_question_and_drops_a_spliced_notice() {
    let root = temp("digest-spliced");
    let home = temp("digest-spliced-home");
    let ws = ws_key(&root);
    transcript(
        &home.join("sessions").join(&ws).join("session-spliced"),
        "{\"type\":\"agent/inbox/spliced\",\"data\":{\"inserted\":[\
         {\"content\":[{\"type\":\"text\",\"text\":\"A spliced question about telegram messages and the builder\"}],\
         \"source\":{\"kind\":\"agent\",\"form\":\"question\"}},\
         {\"content\":[{\"type\":\"text\",\"text\":\"Red/green discipline: no failing test is on record\"}],\
         \"source\":{\"kind\":\"dsh-doublecheck\",\"form\":\"notice\"}}]}}\n",
    );
    let out = run(&root, &home, &["session-digest"]);
    assert!(out.status.success());
    let digest = fs::read_to_string(root.join("memory/logs/session-digest.md")).unwrap();
    assert!(
        digest.contains("A spliced question about telegram"),
        "{digest}"
    );
    assert!(!digest.contains("Red/green discipline"), "{digest}");
}

#[test]
fn session_index_skips_a_json_transcript_with_no_session_event() {
    let root = temp("index-junk");
    fs::create_dir_all(root.join("docs")).unwrap();
    let home = temp("index-junk-home");
    let ws = ws_key(&root);
    transcript(
        &home.join("sessions").join(&ws).join("session-junk"),
        "{\"type\":\"garbage\"}\n",
    );
    transcript(
        &home.join("sessions").join(&ws).join("session-good"),
        "{\"createdAt\":1794796632985,\"type\":\"request/header\"}\n\
         {\"type\":\"session/title\",\"data\":{\"title\":\"The good title\"}}\n",
    );
    let out = run(&root, &home, &["session-index"]);
    assert!(out.status.success());
    let report = fs::read_to_string(root.join("docs/session-index.md")).unwrap();
    assert!(report.contains("**1 sessions**"), "{report}");
    assert!(!report.contains("(untitled)"), "{report}");
}

#[test]
fn log_index_refuses_a_missing_memory_directory() {
    let root = temp("log-missing");
    fs::create_dir_all(root.join("docs")).unwrap();
    let home = temp("log-missing-home");
    let out = run(&root, &home, &["log-index"]);
    assert!(!out.status.success());
    assert!(!root.join("docs/memory-log-index.md").exists());
}

#[test]
fn session_commits_refuses_when_git_cannot_read_the_repository() {
    let root = temp("commits-norepo");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(
        root.join("docs/session-index.md"),
        "| # | Started | Session | Parent | Title | Size | Lines |\n\
         |---|---|---|---|---|---|---|\n\
         | 1 | 2026-11-16 10:00 | `abcdef12` |  | A title | 1 KB | 1 |\n",
    )
    .unwrap();
    let home = temp("commits-norepo-home");
    let out = run(&root, &home, &["session-commits"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("git log failed"));
    assert!(!root.join("docs/session-commits.md").exists());
}

#[test]
fn session_commits_counts_unjoined_commits_when_there_are_no_trailers() {
    let root = temp("commits-unjoined");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(
        root.join("docs/session-index.md"),
        "| # | Started | Session | Parent | Title | Size | Lines |\n\
         |---|---|---|---|---|---|---|\n\
         | 1 | 2026-11-16 10:00 | `abcdef12` |  | A title | 1 KB | 1 |\n",
    )
    .unwrap();
    git(&root, &["init", "-q"]);
    fs::write(root.join("a.txt"), "a").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-q", "-m", "work without a session"]);
    let home = temp("commits-unjoined-home");
    let out = run(&root, &home, &["session-commits", "--days", "30"]);
    assert!(out.status.success());
    let report = fs::read_to_string(root.join("docs/session-commits.md")).unwrap();
    assert!(report.contains("1 unjoined"), "{report}");
    assert!(report.contains("0 sessions joined"), "{report}");
}

#[test]
fn digest_rows_carry_the_id_from_a_plain_session_folder() {
    let root = temp("digest-id");
    let home = temp("digest-id-home");
    let ws = ws_key(&root);
    transcript(
        &home
            .join("sessions")
            .join(&ws)
            .join("1111aaaa-2222-3333-4444-555555555555"),
        "{\"type\":\"session/title\",\"data\":{\"title\":\"Plain folder title\"}}\n\
         {\"type\":\"user/message\",\"data\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"A question about the plain folder session id\"}]}}\n",
    );
    let out = run(&root, &home, &["session-digest"]);
    assert!(out.status.success());
    let digest = fs::read_to_string(root.join("memory/logs/session-digest.md")).unwrap();
    assert!(digest.contains("1111aaaa"), "{digest}");
}

#[test]
fn session_commits_labels_a_joined_session_missing_from_the_index() {
    let root = temp("commits-missing");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(
        root.join("docs/session-index.md"),
        "| # | Started | Session | Parent | Title | Size | Lines |\n\
         |---|---|---|---|---|---|---|\n\
         | 1 | 2026-11-16 10:00 | `deadbeef` |  | Another session | 1 KB | 1 |\n",
    )
    .unwrap();
    git(&root, &["init", "-q"]);
    fs::write(root.join("a.txt"), "a").unwrap();
    git(&root, &["add", "a.txt"]);
    git(
        &root,
        &[
            "commit",
            "-q",
            "-m",
            "First work\n\nDsh-Session: abcdef12-3456",
        ],
    );
    let home = temp("commits-missing-home");
    let out = run(&root, &home, &["session-commits", "--days", "30"]);
    assert!(out.status.success());
    let report = fs::read_to_string(root.join("docs/session-commits.md")).unwrap();
    assert!(report.contains("(not in the index)"), "{report}");
}

// chmod 000 has no Windows meaning: this refusal is a unix contract.
#[cfg(unix)]
#[test]
fn log_index_refuses_an_unreadable_day_file() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp("log-unreadable");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join("memory")).unwrap();
    let day = root.join("memory/2026-01-01.md");
    fs::write(&day, "# 2026-01-01\n\n## A topic\n").unwrap();
    fs::set_permissions(&day, fs::Permissions::from_mode(0o000)).unwrap();
    let home = temp("log-unreadable-home");
    let out = run(&root, &home, &["log-index"]);
    let _ = fs::set_permissions(&day, fs::Permissions::from_mode(0o644));
    assert!(!out.status.success());
    assert!(!root.join("docs/memory-log-index.md").exists());
}

#[test]
fn tier0_history_without_a_store_exits_two() {
    let root = temp("tier0-nostore");
    let home = temp("tier0-nostore-home");
    let out = run(&root, &home, &["tier0", "history"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn session_index_check_never_writes_and_fails_on_an_empty_index() {
    let root = temp("index-check");
    fs::create_dir_all(root.join("docs")).unwrap();
    let home = temp("index-check-home");
    let out = run(&root, &home, &["session-index", "--check"]);
    assert!(!out.status.success());
    assert!(!root.join("docs/session-index.md").exists());
}

#[test]
fn digest_refuses_an_empty_digest_when_a_transcript_exists() {
    let root = temp("digest-empty");
    let home = temp("digest-empty-home");
    let ws = ws_key(&root);
    transcript(
        &home.join("sessions").join(&ws).join("session-garbage"),
        "not json at all\n",
    );
    let out = run(&root, &home, &["session-digest"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("refusing"));
    assert!(!root.join("memory/logs/session-digest.md").exists());
}

#[test]
fn session_commits_uses_iso_dates_and_labels_machine_commits() {
    let root = temp("commits");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(
        root.join("docs/session-index.md"),
        "| # | Started | Session | Parent | Title | Size | Lines |\n\
         |---|---|---|---|---|---|---|\n\
         | 1 | 2026-11-16 10:00 | `abcdef12` |  | The session title | 1 KB | 1 |\n",
    )
    .unwrap();
    git(&root, &["init", "-q"]);
    fs::write(root.join("a.txt"), "a").unwrap();
    git(&root, &["add", "a.txt"]);
    git(
        &root,
        &[
            "commit",
            "-q",
            "-m",
            "First work\n\nDsh-Session: abcdef12-3456",
        ],
    );
    fs::write(root.join("b.txt"), "b").unwrap();
    git(&root, &["add", "b.txt"]);
    git(
        &root,
        &["commit", "-q", "-m", "Auto backup 2026-11-16 03:00"],
    );
    let home = temp("commits-home");
    let out = run(&root, &home, &["session-commits", "--days", "30"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = fs::read_to_string(root.join("docs/session-commits.md")).unwrap();
    assert!(report.contains("The session title"), "{report}");
    assert!(report.contains("1 machine"), "{report}");
    assert!(report.contains("0 unjoined"), "{report}");
    let row = report
        .lines()
        .find(|l| l.contains("The session title"))
        .unwrap();
    assert!(
        row.split('|').nth(4).unwrap().trim().starts_with("20"),
        "the joined date is not ISO: {row}"
    );
}

#[test]
fn session_commits_refuses_an_index_with_no_titles() {
    let root = temp("commits-empty");
    fs::create_dir_all(root.join("docs")).unwrap();
    git(&root, &["init", "-q"]);
    fs::write(root.join("a.txt"), "a").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-q", "-m", "work with no session"]);
    let home = temp("commits-empty-home");
    let out = run(&root, &home, &["session-commits", "--days", "30"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no titles"));
    assert!(!root.join("docs/session-commits.md").exists());
}

#[test]
fn the_daily_log_index_lists_the_day_and_its_topics() {
    let root = temp("logs");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join("memory")).unwrap();
    fs::write(
        root.join("memory/2026-01-01.md"),
        "# 2026-01-01\n\n## A topic worth indexing\n",
    )
    .unwrap();
    let home = temp("logs-home");
    let out = run(&root, &home, &["log-index"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = fs::read_to_string(root.join("docs/memory-log-index.md")).unwrap();
    assert!(report.contains("2026-01-01"), "{report}");
    assert!(report.contains("A topic worth indexing"), "{report}");
}

#[test]
fn check_selftest_proves_the_unparseable_row_finding() {
    let root = temp("selftest");
    let home = temp("selftest-home");
    let out = run(&root, &home, &["check", "--selftest"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("selftest PASS"));
}

#[test]
fn find_runs_the_finder_and_reindexes() {
    if !optimisemd_core::util::shell_available() {
        return no_shell("find running the finder stub");
    }
    let root = temp("find");
    let home = temp("find-home");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let stub = root.join("bin/dsh-belfind");
    std::fs::write(&stub, "#!/usr/bin/env bash\nprintf 'stub %s\\n' \"$*\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::create_dir_all(root.join("memory/logs")).unwrap();
    for f in ["session-digest.md", "memento-export.md", "find-index.json"] {
        std::fs::write(root.join("memory/logs").join(f), "").unwrap();
    }
    let out = run(&root, &home, &["find", "hello", "world"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("stub find hello world"), "{text}");
    let out = run(&root, &home, &["find", "--reindex"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("stub index"));
}

// A finder that is only a script (no executable bit on unix, no runnable type on Windows) is run
// through a shell: the direct spawn fails and the fallback must still answer (CI caught this on
// Windows, where a script finder could not be spawned at all).
#[test]
fn find_runs_a_finder_that_is_only_a_script() {
    if !optimisemd_core::util::shell_available() {
        return no_shell("find running a script-only finder");
    }
    let root = temp("find-script-only");
    let home = temp("find-script-only-home");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    // Deliberately no chmod: on unix this is not executable, on Windows it is not runnable.
    std::fs::write(
        root.join("bin/dsh-belfind"),
        "#!/usr/bin/env bash\nprintf 'stub %s\\n' \"$*\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("memory/logs")).unwrap();
    for f in ["session-digest.md", "memento-export.md", "find-index.json"] {
        std::fs::write(root.join("memory/logs").join(f), "").unwrap();
    }
    let out = run(&root, &home, &["find", "hello", "world"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("stub find hello world"), "{text}");
}

#[test]
fn find_refuses_without_the_finder_binary() {
    let root = temp("find-bare");
    let home = temp("find-bare-home");
    let out = run(&root, &home, &["find", "x"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no dsh-belfind"));
}

// The historical-name dispatch runs through a symlink and a bash stub: a unix contract.
#[test]
fn the_historical_wrapper_names_dispatch() {
    if !optimisemd_core::util::shell_available() {
        return no_shell("the historical wrapper dispatch");
    }
    let root = temp("alias");
    #[cfg(unix)]
    let home = temp("alias-home");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let stub = root.join("bin/dsh-belfind");
    std::fs::write(&stub, "#!/usr/bin/env bash\nprintf 'stub %s\\n' \"$*\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir_all(root.join("memory/logs")).unwrap();
        for f in ["session-digest.md", "memento-export.md", "find-index.json"] {
            std::fs::write(root.join("memory/logs").join(f), "").unwrap();
        }
        let link = root.join("find.sh");
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_dsh-optimisemd"), &link).unwrap();
        let out = std::process::Command::new(&link)
            .arg("hello")
            .current_dir(&root)
            .env("DSH_HOME", &home)
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("stub find hello"));
    }
}
