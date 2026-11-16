//! The save gate on a host with no usable POSIX shell.
//!
//! The test is unix-only: on Windows the gate deliberately looks for Git's bash in its fixed
//! program locations, so on a machine with Git installed a "no shell" state cannot be produced
//! without uninstalling it. Where the state cannot exist, the refusal message is still covered by
//! the unit test for the message, and the rollback by the save-with-a-failing-gate test.
//!
//! This lives in its own test binary because it empties `PATH` for the whole process, and any other
//! test running beside it in the same process could then fail to spawn a shell. One test, one
//! process, no neighbours to disturb.

use optimisemd_core::Repo;

fn root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("optimisemd-noshell-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("docs/memory-details")).unwrap();
    std::fs::create_dir_all(dir.join("memory")).unwrap();
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::create_dir_all(dir.join("empty-path")).unwrap();
    dir
}

#[cfg(unix)]
#[test]
fn a_save_with_no_shell_says_the_check_could_not_run_and_rolls_back() {
    let root = root("save");
    let memory = "# MEMORY.md\n\n## Subject facts\n- **An existing row.** (2026-01-01) — `docs/memory-details/01.md`\n";
    std::fs::write(root.join("MEMORY.md"), memory).unwrap();
    // Not executable, so it cannot be spawned directly: only a shell could run it, and this test
    // takes every shell out of PATH.
    std::fs::write(
        root.join("scripts/check-memory-index.sh"),
        "#!/bin/sh\nexit 1\n",
    )
    .unwrap();

    let old_path = std::env::var_os("PATH");
    let old_shell = std::env::var_os("SHELL");
    std::env::set_var("PATH", root.join("empty-path"));
    std::env::remove_var("SHELL");
    let result = Repo::new(&root).save_checked(
        "A fact that will be refused.",
        "2026-11-17",
        "docs/memory-details/01.md",
        Some("Subject facts"),
        None,
        None,
        None,
    );
    match old_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }
    if let Some(value) = old_shell {
        std::env::set_var("SHELL", value);
    }

    let err = result
        .expect_err("a host with no shell must refuse the save")
        .to_string();
    assert!(
        err.contains("could not run"),
        "the reason must be a missing shell, not a failed check: {err}"
    );
    assert!(err.contains("Git for Windows"), "{err}");
    assert!(!err.contains("index check failed"), "{err}");
    assert_eq!(
        std::fs::read_to_string(root.join("MEMORY.md")).unwrap(),
        memory,
        "the refused save must leave MEMORY.md exactly as it was"
    );
    assert!(
        !root.join("memory/2026-11-17.md").exists(),
        "the refused save must not leave a day file behind"
    );
    let audit =
        std::fs::read_to_string(root.join("memory/logs/repo-writes.log")).unwrap_or_default();
    assert!(
        audit.contains("rolled-back"),
        "the rollback must be audited, not silent: {audit}"
    );
}
