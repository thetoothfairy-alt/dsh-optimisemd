//! Self-repair — find the classes of maintenance fault that separate nightly steps miss, and rebuild the
//! generated reports that can be rebuilt deterministically.
//!
//! The 2026-11-21 review found four: a stale generated report, a guard that fails while the store is
//! fine (a date-blind append-only check), a guard that cannot prove itself (a truncated parity script),
//! and discovery gaps. The report repairs are safe to apply; the guard faults are reported with the
//! reason and the next action, never silently passed.

use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use chrono::{Duration, NaiveDate};
use optimisemd_core::Store;

use crate::{counts, reports};

#[derive(Default)]
struct Reporter {
    fixed: usize,
    open: usize,
}

impl Reporter {
    fn ok(&self, name: &str, detail: impl std::fmt::Display) {
        println!("  [ok]   {name}: {detail}");
    }
    fn fixed(&mut self, name: &str, detail: impl std::fmt::Display) {
        self.fixed += 1;
        println!("  [fix]  {name}: {detail}");
    }
    fn open(&mut self, name: &str, detail: impl std::fmt::Display) {
        self.open += 1;
        println!("  [open] {name}: {detail}");
    }
}

pub fn run(root: &Path, fix: bool) -> Result<i32, Box<dyn Error>> {
    let mut r = Reporter::default();
    check_store(root, fix, &mut r)?;
    check_index(root, &mut r)?;
    check_session_index(root, fix, &mut r)?;
    check_digest(root, fix, &mut r)?;
    check_session_commits(root, fix, &mut r)?;
    check_log_index(root, fix, &mut r)?;
    check_counts(root, fix, &mut r)?;
    check_finder(root, fix, &mut r)?;
    check_nightly(root, &mut r);
    check_discovery(root, &mut r);
    check_store_separation(root, &mut r);
    check_parity(root, &mut r);
    println!();
    println!("self-repair: {} repaired · {} open", r.fixed, r.open);
    Ok(if r.open == 0 { 0 } else { 1 })
}

fn check_store(root: &Path, fix: bool, r: &mut Reporter) -> Result<(), Box<dyn Error>> {
    let Some(src) = crate::store_path_read() else {
        r.ok("store", "no tier-0 store yet");
        return Ok(());
    };
    let store = match Store::open_readonly(&src) {
        Ok(s) => s,
        Err(e) => {
            r.open("store", format!("cannot open {}: {e}", src.display()));
            return Ok(());
        }
    };
    let entries = match store.list_entries() {
        Ok(e) => e,
        Err(e) => {
            r.open("store", format!("cannot read entries: {e}"));
            return Ok(());
        }
    };
    let export = root.join("memory/logs/memento-export.md");
    let rows = export_rows(&export);
    if rows == entries.len() {
        r.ok(
            "export",
            format!("matches the store ({} entries)", entries.len()),
        );
    } else if !fix {
        r.open(
            "export",
            format!(
                "lists {rows} of {} stored entries — run with --fix or `export`",
                entries.len()
            ),
        );
    } else {
        match crate::export_inner(root) {
            Ok(n) => {
                let after = export_rows(&export);
                if after == n {
                    r.fixed("export", format!("rewrote {n} entries"));
                } else {
                    r.open(
                        "export",
                        format!("rewrote {after} rows for {n} entries; still short"),
                    );
                }
            }
            Err(e) => r.open("export", format!("cannot rewrite: {e}")),
        }
    }
    let user = store.usage("user", "user-global")?;
    let agent = store.usage("agent", "workspace")?;
    if user > store.cap("user") || agent > store.cap("agent") {
        r.open(
            "store",
            format!(
                "over its cap: user {user}/{}, agent {agent}/{}",
                store.cap("user"),
                store.cap("agent")
            ),
        );
    } else {
        r.ok(
            "store",
            format!(
                "user {user}/{} · agent {agent}/{}",
                store.cap("user"),
                store.cap("agent")
            ),
        );
    }
    Ok(())
}

fn export_rows(path: &Path) -> usize {
    fs::read_to_string(path)
        .map(|t| t.lines().filter(|l| l.starts_with("- **")).count())
        .unwrap_or(0)
}

/// The tool's own index check is a first-class health signal: a stale pointer or an unparseable row is
/// exactly the kind of quiet fault a self-repair should surface.
fn check_index(root: &Path, r: &mut Reporter) -> Result<(), Box<dyn Error>> {
    if !root.join("MEMORY.md").exists() {
        r.ok("index", "no MEMORY.md");
        return Ok(());
    }
    let code = crate::memindex::run(root, Vec::new())?;
    if code == 0 {
        r.ok("index", "0 findings");
    } else {
        r.open("index", "the index check reports findings (above)");
    }
    Ok(())
}

fn dsh_home() -> String {
    std::env::var("DSH_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/.dsh", std::env::var("HOME").unwrap_or_default()))
}

fn has_transcripts(root: &Path) -> bool {
    !reports::transcript_files(&dsh_home(), &reports::ws_key(root)).is_empty()
}

fn check_session_index(root: &Path, fix: bool, r: &mut Reporter) -> Result<(), Box<dyn Error>> {
    let out = root.join("docs/session-index.md");
    if !root.join("docs").is_dir() {
        r.ok("session index", "no docs directory");
        return Ok(());
    }
    if !has_transcripts(root) && !out.exists() {
        r.ok("session index", "no sessions yet");
        return Ok(());
    }
    let state = reports::session_index(root, true)?;
    if state == 0 {
        r.ok("session index", "current");
        return Ok(());
    }
    if !fix {
        r.open("session index", "stale or missing — run with --fix");
        return Ok(());
    }
    let code = reports::session_index(root, false)?;
    if code == 0 {
        r.fixed("session index", "rebuilt");
    } else {
        r.open(
            "session index",
            "the rebuild was refused; see the message above",
        );
    }
    Ok(())
}

fn check_digest(root: &Path, fix: bool, r: &mut Reporter) -> Result<(), Box<dyn Error>> {
    let index = root.join("docs/session-index.md");
    if !index.exists() {
        r.ok("digest", "no session index yet");
        return Ok(());
    }
    let sessions = count_index_sessions(&index);
    if sessions == 0 {
        r.ok("digest", "the index has no sessions");
        return Ok(());
    }
    let digest = root.join("memory/logs/session-digest.md");
    let rows = count_digest_rows(&digest);
    if rows == sessions {
        r.ok("digest", format!("{rows} rows, matches the index"));
        return Ok(());
    }
    if !fix {
        r.open(
            "digest",
            format!("{rows} rows for {sessions} indexed sessions — run with --fix"),
        );
        return Ok(());
    }
    match reports::session_digest(root) {
        Ok(_) => {
            let after = count_digest_rows(&digest);
            if after == sessions {
                r.fixed("digest", format!("rebuilt {after} rows"));
            } else {
                r.open(
                    "digest",
                    format!("rebuilt {after} rows, still {sessions} expected"),
                );
            }
        }
        Err(e) => r.open("digest", format!("cannot rebuild: {e}")),
    }
    Ok(())
}

fn count_index_sessions(path: &Path) -> usize {
    fs::read_to_string(path)
        .map(|t| {
            t.lines()
                .filter(|l| {
                    l.split('|')
                        .nth(1)
                        .and_then(|c| c.trim().parse::<usize>().ok())
                        .is_some()
                })
                .count()
        })
        .unwrap_or(0)
}

fn count_digest_rows(path: &Path) -> usize {
    fs::read_to_string(path)
        .map(|t| t.lines().filter(|l| l.starts_with("- **")).count())
        .unwrap_or(0)
}

/// The commit join is only meaningful in a git workspace, and it says when the index it read lacked a
/// title. Rebuild after the index; if the gap survives the rebuild it is a real missing title, not a
/// stale report.
fn check_session_commits(root: &Path, fix: bool, r: &mut Reporter) -> Result<(), Box<dyn Error>> {
    let index = root.join("docs/session-index.md");
    if !index.exists() || !root.join(".git").exists() {
        r.ok("session commits", "no git index to join");
        return Ok(());
    }
    let out = root.join("docs/session-commits.md");
    let text = fs::read_to_string(&out).unwrap_or_default();
    let missing = text.contains("Joined sessions with no title");
    let stale = !out.exists() || mtime(&out).is_none_or(|t| mtime(&index).is_some_and(|i| t < i));
    if !stale && !missing {
        r.ok("session commits", "current");
        return Ok(());
    }
    if !fix {
        r.open(
            "session commits",
            if missing {
                "a joined session has no title in the index — rebuild after the index"
            } else {
                "older than the index — run with --fix"
            },
        );
        return Ok(());
    }
    match reports::session_commits(root, "30") {
        Ok(0) => {
            let after = fs::read_to_string(&out).unwrap_or_default();
            if after.contains("Joined sessions with no title") {
                r.open(
                    "session commits",
                    "rebuilt, but a joined session still has no title in the index",
                );
            } else {
                r.fixed("session commits", "rebuilt");
            }
        }
        Ok(_) => r.open(
            "session commits",
            "the rebuild was refused; see the message above",
        ),
        Err(e) => r.open("session commits", format!("cannot rebuild: {e}")),
    }
    Ok(())
}

fn check_log_index(root: &Path, fix: bool, r: &mut Reporter) -> Result<(), Box<dyn Error>> {
    let memory = root.join("memory");
    if !memory.is_dir() || !root.join("docs").is_dir() {
        r.ok("log index", "no docs/memory tree");
        return Ok(());
    }
    let newest = newest_daily_log(&memory);
    let Some(newest) = newest else {
        r.ok("log index", "no daily logs");
        return Ok(());
    };
    let out = root.join("docs/memory-log-index.md");
    let stale = !out.exists() || mtime(&out).is_none_or(|t| t < newest);
    if !stale {
        r.ok("log index", "current");
        return Ok(());
    }
    if !fix {
        r.open("log index", "older than the daily logs — run with --fix");
        return Ok(());
    }
    match reports::memory_log_index(root) {
        Ok(0) => r.fixed("log index", "rebuilt"),
        Ok(_) => r.open(
            "log index",
            "the rebuild was refused; see the message above",
        ),
        Err(e) => r.open("log index", format!("cannot rebuild: {e}")),
    }
    Ok(())
}

fn newest_daily_log(memory: &Path) -> Option<SystemTime> {
    let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}\.md$").ok()?;
    fs::read_dir(memory)
        .ok()?
        .flatten()
        .filter(|e| re.is_match(&e.file_name().to_string_lossy()))
        .filter_map(|e| e.metadata().and_then(|m| m.modified()).ok())
        .max()
}

fn check_counts(root: &Path, fix: bool, r: &mut Reporter) -> Result<(), Box<dyn Error>> {
    if !root.join(counts::DOC).exists() {
        r.ok("counts", "no measured block document");
        return Ok(());
    }
    let before = match counts::run(root, false) {
        Ok(c) => c,
        Err(e) => {
            r.open("counts", format!("cannot check: {e}"));
            return Ok(());
        }
    };
    if before == 0 {
        r.ok("counts", "in step with the tree");
        return Ok(());
    }
    if !fix {
        r.open("counts", "the measured block is stale — run with --fix");
        return Ok(());
    }
    let wrote = counts::run(root, true);
    let after = counts::run(root, false);
    match (wrote, after) {
        (Ok(0), Ok(0)) => r.fixed("counts", "rewrote the measured block"),
        (Err(e), _) => r.open("counts", format!("cannot rewrite: {e}")),
        _ => r.open("counts", "still stale after the rewrite"),
    }
    Ok(())
}

fn check_finder(root: &Path, fix: bool, r: &mut Reporter) -> Result<(), Box<dyn Error>> {
    if crate::finder(root).is_none() {
        r.ok("finder", "not installed; finder checks skipped");
        return Ok(());
    }
    let index = root.join("memory/logs/find-index.json");
    let Some(newest) = newest_source(root) else {
        r.ok("finder", "no markdown sources");
        return Ok(());
    };
    let stale = !index.exists() || mtime(&index).is_none_or(|t| t < newest);
    if !stale {
        r.ok("finder", "the index is current");
        return Ok(());
    }
    if !fix {
        r.open(
            "finder",
            "the index is older than its sources — run with --fix",
        );
        return Ok(());
    }
    let code = crate::index(root)?;
    if code == 0 && index.exists() {
        r.fixed("finder", "rebuilt the index");
    } else {
        r.open("finder", "the finder exited without writing an index");
    }
    Ok(())
}

fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The newest markdown the finder reads. Generated reports and docs are both sources: an edit to
/// either makes the ranked index older than what it claims to rank.
fn newest_source(root: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    for rel in ["docs", "memory", "skills"] {
        walk_md(&root.join(rel), &mut newest);
    }
    for rel in ["AGENTS.md", "MEMORY.md"] {
        if let Some(t) = mtime(&root.join(rel)) {
            newest = Some(newest.map_or(t, |n| n.max(t)));
        }
    }
    newest
}

fn walk_md(dir: &Path, newest: &mut Option<SystemTime>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if matches!(
            name.as_str(),
            ".git" | "target" | "node_modules" | "sessions-archive"
        ) {
            continue;
        }
        let Ok(ft) = e.file_type() else {
            continue;
        };
        let path = e.path();
        if ft.is_dir() {
            walk_md(&path, newest);
        } else if name.ends_with(".md") {
            if let Some(t) = mtime(&path) {
                *newest = Some(newest.map_or(t, |n| n.max(t)));
            }
        }
    }
}

fn check_nightly(root: &Path, r: &mut Reporter) {
    let dir = root.join("memory/nightly");
    let Ok(entries) = fs::read_dir(&dir) else {
        r.ok("nightly", "no nightly log yet");
        return;
    };
    let mut logs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("log"))
        .collect();
    logs.sort();
    let Some(latest) = logs.last() else {
        r.ok("nightly", "no nightly log yet");
        return;
    };
    let name = latest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let text = fs::read_to_string(latest).unwrap_or_default();
    // The log names a failed step on a `FAIL <label> (exit N)` line and prints that step's own
    // findings above it, so a finding belongs to the step label that follows it.
    let mut steps: Vec<(String, usize)> = Vec::new();
    let mut untagged = 0usize;
    for l in text.lines() {
        let Some(rest) = l.trim().strip_prefix("FAIL ") else {
            continue;
        };
        let rest = rest.trim();
        match rest.split_once(" (exit") {
            Some((label, _)) => steps.push((label.trim().to_string(), 0)),
            None if rest.starts_with('[') => {
                if let Some((_, n)) = steps.last_mut() {
                    *n += 1;
                } else {
                    untagged += 1;
                }
            }
            None if !rest.is_empty() => untagged += 1,
            None => {}
        }
    }
    if steps.is_empty() {
        if untagged == 0 && !text.contains("result: FAILED") {
            r.ok("nightly", format!("{name}: no failures"));
        } else {
            r.open(
                "nightly",
                format!("{name}: {untagged} failure line(s) with no step to re-run"),
            );
        }
        return;
    }
    // Re-check each failed step against this tree: the log is a record of last night, not a claim
    // about now. The step's command is read back from the nightly script it came from.
    let script =
        fs::read_to_string(root.join("scripts/nightly-maintenance.sh")).unwrap_or_default();
    let mut cmds: HashMap<&str, &str> = HashMap::new();
    for line in script.lines() {
        let Some(rest) = line.trim_start().strip_prefix("step ") else {
            continue;
        };
        let Some(rest) = rest.strip_prefix('"') else {
            continue;
        };
        let Some((label, cmd)) = rest.split_once('"') else {
            continue;
        };
        cmds.entry(label).or_insert(cmd.trim());
    }
    let mut fixed: Vec<String> = Vec::new();
    let mut still: Vec<String> = Vec::new();
    for (label, _) in &steps {
        match cmds.get(label.as_str()) {
            Some(cmd) if is_runnable(cmd) => {
                let pass = std::process::Command::new("bash")
                    .args(["-c", cmd])
                    .current_dir(root)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if pass {
                    fixed.push(label.clone());
                } else {
                    still.push(label.clone());
                }
            }
            Some(_) => still.push(format!("{label} (a shell helper, not re-run here)")),
            None => still.push(format!("{label} (no longer in the nightly script)")),
        }
    }
    if still.is_empty() {
        r.ok(
            "nightly",
            format!(
                "{name}: {} failure(s) listed, all fixed since ({})",
                steps.len(),
                fixed.join(", ")
            ),
        );
    } else {
        r.open("nightly", format!("{name}: {}", still.join(", ")));
    }
}

/// A step command the re-check may run: a path or `bash ...`, never a shell function.
fn is_runnable(cmd: &str) -> bool {
    cmd.split_whitespace()
        .next()
        .map(|t| t == "bash" || t == "sh" || t.contains('/'))
        .unwrap_or(false)
}

fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn check_discovery(root: &Path, r: &mut Reporter) {
    let script = root.join("scripts/check-discovery-coverage.sh");
    if !script.exists() {
        r.ok("discovery", "the coverage check is not present");
        return;
    }
    let out = match optimisemd_core::util::run_script(&script, root) {
        Ok(o) => o,
        Err(e) => {
            r.open("discovery", format!("cannot run the coverage check: {e}"));
            return;
        }
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        let line = last_line(&text);
        let line = line.strip_prefix("discovery: ").unwrap_or(&line);
        r.ok("discovery", line);
        return;
    }
    let unlisted = text
        .lines()
        .filter(|l| l.starts_with("RUNBOOK not in"))
        .count();
    let proof = text.lines().filter(|l| l.starts_with("proof gap:")).count();
    r.open(
        "discovery",
        format!(
            "{unlisted} runbook(s) unlisted, {proof} without a stated proof — {}",
            last_line(&text)
        ),
    );
}

fn last_commit_date(root: &Path, path: &str) -> Option<String> {
    let out = Command::new("git")
        .args([
            "-C",
            &root.to_string_lossy(),
            "log",
            "-1",
            "--format=%cs",
            "--",
            path,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn within_window(file_date: &str, commit_date: &str) -> Option<bool> {
    let f = NaiveDate::parse_from_str(file_date, "%Y-%m-%d").ok()?;
    let c = NaiveDate::parse_from_str(commit_date, "%Y-%m-%d").ok()?;
    Some(c <= f + Duration::days(7))
}

fn check_store_separation(root: &Path, r: &mut Reporter) {
    let script = root.join("scripts/check-store-separation.sh");
    if !script.exists() {
        r.ok("store separation", "the separation check is not present");
        return;
    }
    let out = match optimisemd_core::util::run_script(&script, root) {
        Ok(o) => o,
        Err(e) => {
            r.open("store separation", format!("cannot run the check: {e}"));
            return;
        }
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        r.ok("store separation", last_line(&text));
        return;
    }
    let (mut within, mut later, mut unknown) = (0usize, 0usize, 0usize);
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("[2] ") else {
            continue;
        };
        let Some((path, _)) = rest.split_once(':') else {
            unknown += 1;
            continue;
        };
        let date = path
            .strip_prefix("memory/")
            .and_then(|p| p.strip_suffix(".md"))
            .unwrap_or("");
        match last_commit_date(root, path)
            .as_deref()
            .and_then(|c| within_window(date, c))
        {
            Some(true) => within += 1,
            Some(false) => later += 1,
            None => unknown += 1,
        }
    }
    let summary = last_line(&text);
    if within > 0 && later == 0 && unknown == 0 {
        r.open(
            "store separation",
            format!(
                "{within} finding(s) from an edit within the 7-day window — the guard is date-blind, not a store violation; {summary}"
            ),
        );
    } else if later > 0 {
        r.open(
            "store separation",
            format!(
                "{later} deletion(s) after the 7-day mark — a real append-only violation; {summary}"
            ),
        );
    } else {
        r.open("store separation", summary);
    }
}

fn check_parity(root: &Path, r: &mut Reporter) {
    let path = root.join("tools/wschk/tests/parity.sh");
    if !path.exists() {
        r.ok("parity", "the harness is not present");
        return;
    }
    let text = fs::read_to_string(&path).unwrap_or_default();
    let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
    let verdict = text.contains("PARITY OK");
    let comparisons = text.contains("identical");
    if !verdict || !comparisons {
        r.open(
            "parity",
            format!(
                "{lines} non-empty lines; verdict {}, comparisons {} — truncated or vacuous; parity is unproven",
                if verdict { "present" } else { "missing" },
                if comparisons { "present" } else { "missing" }
            ),
        );
    } else {
        r.ok("parity", "the harness has comparisons and a verdict");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_inside_the_seven_day_window_is_not_an_append_only_violation() {
        assert_eq!(within_window("2026-11-12", "2026-11-12"), Some(true));
        assert_eq!(within_window("2026-11-12", "2026-11-19"), Some(true));
        assert_eq!(within_window("2026-11-12", "2026-11-20"), Some(false));
        assert_eq!(within_window("not-a-date", "2026-11-12"), None);
    }

    #[test]
    fn index_rows_are_counted_by_their_numeric_first_cell() {
        let dir = std::env::temp_dir().join(format!("optimisemd-sr-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("index.md");
        fs::write(
            &p,
            "| # | Started |\n|---|---|\n| 1 | today |\n| 2 | today |\n",
        )
        .unwrap();
        assert_eq!(count_index_sessions(&p), 2);
        fs::write(&p, "| # | Started |\n|---|---|\n").unwrap();
        assert_eq!(count_index_sessions(&p), 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
