//! dsh-optimisemd — the command line the workspace's nightlies call. One binary owns promotion, the checks gate, the
//! tier-0 export and the finder refresh, so the shell scripts become either thin aliases or disappear.

use clap::{Args, CommandFactory, Parser, Subcommand};
use optimisemd_core::{Promoter, Store};
use std::path::{Path, PathBuf};
use std::process::Command;

mod counts;
mod landmine;
mod memindex;
mod reports;
mod self_repair;
mod slim;

#[derive(Parser)]
#[command(
    name = "dsh-optimisemd",
    version,
    about = "Memory management for the workspace: promote, check, export, index."
)]
struct Cli {
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Report demote candidates and promote candidates; apply the demotions with --apply.
    Promote(PromoteArgs),
    /// Check the memory indexes (MEMORY.md and docs/lesson-details.md unless paths are given).
    Check {
        /// Files to check; empty means the workspace default.
        targets: Vec<String>,
        /// Run the self-test: a temp fixture must still produce the unparseable-row finding.
        #[arg(long)]
        selftest: bool,
    },
    /// Write memory/logs/memento-export.md from a read-only copy of the tier-0 store.
    Export,
    /// Rebuild the ranked finder index (delegates to the workspace's finder).
    Index,
    /// Ask the ranked finder; regenerates a missing corpus first. --reindex rebuilds the index instead.
    Find {
        /// Rebuild the index instead of searching.
        #[arg(long)]
        reindex: bool,
        /// The question (plain words); passed to the finder.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        query: Vec<String>,
    },
    /// Measure the always-read markdown surface against a baseline commit (writes docs/md-slimming-report.md).
    Slim(SlimCliArgs),
    /// Measure the counts in docs/memory-process.md; --write rewrites the block.
    Counts {
        #[arg(long)]
        write: bool,
    },
    /// Rebuild docs/session-index.md (every session, newest first); --check fails when the file is stale.
    SessionIndex {
        #[arg(long)]
        check: bool,
    },
    /// Rebuild docs/session-commits.md over the last --days.
    SessionCommits {
        #[arg(long, default_value_t = 30)]
        days: u32,
    },
    /// Rebuild memory/logs/session-digest.md (one row per session, what it was asked).
    SessionDigest,
    /// Rebuild docs/memory-log-index.md (the daily logs' topics).
    LogIndex,
    /// Find stale reports and broken maintenance guards; --fix rebuilds the reports that are safe to rebuild.
    SelfRepair {
        /// Rebuild the stale reports instead of only reporting them.
        #[arg(long)]
        fix: bool,
    },
    /// Record a landmine: one line in its part, its reproduction under §slug, the context guard as the gate.
    Landmine(LandmineArgs),
    /// Tier-0 maintenance, the replacement for memento-maintenance.sh: list, add, replace, remove, query, verify.
    Tier0(Tier0Args),
}

#[derive(Args)]
struct Tier0Args {
    #[command(subcommand)]
    cmd: Tier0Cmd,
}

#[derive(Subcommand)]
enum Tier0Cmd {
    List,
    Add(AddArgs),
    Replace(ReplaceArgs),
    Remove(RemoveArgs),
    Query {
        text: String,
    },
    Verify {
        #[arg(long)]
        report_only: bool,
    },
    /// List the store's audit rows (its change log), newest first.
    History {
        /// How many rows, newest first.
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    Merge(MergeArgs),
    /// Import entries from a legacy dsh-memento database into the plugin's own store.
    Import {
        #[arg(long)]
        from: String,
    },
}

#[derive(Args)]
struct LandmineArgs {
    #[command(subcommand)]
    cmd: LandmineCmd,
}

#[derive(Subcommand)]
enum LandmineCmd {
    Add {
        #[arg(long)]
        part: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        context: String,
        #[arg(long)]
        slug: String,
        #[arg(long)]
        detail: String,
    },
}

#[derive(Args)]
struct SlimCliArgs {
    #[arg(long)]
    baseline: Option<String>,
}

#[derive(Args)]
struct MergeArgs {
    #[arg(long)]
    text: String,
    #[arg(long, default_value = "")]
    tags: String,
    #[arg(long = "match", required = true)]
    matches: Vec<String>,
    #[arg(long, default_value = "user")]
    track: String,
    #[arg(long, default_value = "user-global")]
    scope: String,
}

#[derive(Args)]
struct AddArgs {
    #[arg(long, default_value = "agent")]
    track: String,
    #[arg(long, default_value = "workspace")]
    scope: String,
    #[arg(long)]
    text: String,
    #[arg(long, default_value = "")]
    tags: String,
    #[arg(long, default_value = "cli")]
    source: String,
}

#[derive(Args)]
struct ReplaceArgs {
    #[arg(long)]
    match_: String,
    #[arg(long)]
    text: String,
}

#[derive(Args)]
struct RemoveArgs {
    #[arg(long)]
    match_: String,
}

#[derive(Args)]
struct PromoteArgs {
    #[arg(long, default_value_t = 180)]
    days: u32,
    #[arg(long)]
    apply: bool,
    /// With --apply: show what would move and write nothing.
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

fn main() {
    // The historical names survive as symlinks; when invoked through one, insert the subcommand those names mean.
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if let Some(name) = args
        .first()
        .and_then(|a| Path::new(a).file_name())
        .and_then(|n| n.to_str())
    {
        let sub: Option<&[&str]> = match name {
            "check-memory-index" | "check-memory-index.sh" => Some(&["check"]),
            "check-memento" | "check-memento.sh" => Some(&["tier0", "verify"]),
            "memory-counts" | "memory-counts.sh" => Some(&["counts"]),
            "find" | "find.sh" => Some(&["find"]),
            _ => None,
        };
        if let Some(sub) = sub {
            let mut next: Vec<std::ffi::OsString> = vec![args.remove(0)];
            next.extend(sub.iter().map(std::ffi::OsString::from));
            next.extend(args);
            args = next;
        }
    }
    let cli = Cli::parse_from(args);
    let root = reports::canonical_root(&cli.root);
    let code = match run(&root, cli.cmd) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("dsh-optimisemd: {e}");
            1
        }
    };
    std::process::exit(code);
}

fn run(root: &Path, cmd: Cmd) -> Result<i32, Box<dyn std::error::Error>> {
    match cmd {
        Cmd::Promote(a) => promote(root, a),
        Cmd::Check { targets, selftest } => {
            if selftest {
                memindex::selftest()
            } else {
                memindex::run(root, targets)
            }
        }
        Cmd::Export => export(root),
        Cmd::Index => index(root),
        Cmd::Find { reindex, query } => find(root, reindex, query),
        Cmd::Slim(a) => slim::run(
            root,
            slim::SlimArgs {
                baseline: a.baseline.as_deref(),
            },
        ),
        Cmd::Counts { write } => counts::run(root, write),
        Cmd::SessionIndex { check } => reports::session_index(root, check),
        Cmd::SessionCommits { days } => reports::session_commits(root, &days.to_string()),
        Cmd::SessionDigest => reports::session_digest(root),
        Cmd::LogIndex => reports::memory_log_index(root),
        Cmd::SelfRepair { fix } => self_repair::run(root, fix),
        Cmd::Landmine(a) => match a.cmd {
            LandmineCmd::Add {
                part,
                text,
                context,
                slug,
                detail,
            } => landmine::add(
                root,
                landmine::AddArgs {
                    part: &part,
                    text: &text,
                    context: &context,
                    slug: &slug,
                    detail: &detail,
                },
            ),
        },
        Cmd::Tier0(t) => tier0(root, t),
    }
}

fn promote(root: &Path, a: PromoteArgs) -> Result<i32, Box<dyn std::error::Error>> {
    let promoter = Promoter::new(root);
    if !a.apply {
        let report = promoter.report(a.days)?;
        if a.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!(
                "tier-1 rows: {} · window: {} days",
                report.rows, report.days
            );
            println!("DEMOTE candidates: {}", report.demote.len());
            for r in &report.demote {
                println!("   ({}) {}  → {}", r.date, r.text, r.pointer);
            }
            println!(
                "PROMOTE candidates (tier-2 subject active, no row): {}",
                report.promote.len()
            );
            for p in &report.promote {
                println!("   {} §{}", p.part, p.slug);
            }
            let dir = root.join("memory/logs");
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("promotion-candidates.md");
            let mut body = String::from("# Promotion candidates — the nightly report\n\n");
            body.push_str(&format!(
                "Generated by `bin/dsh-optimisemd promote` at {} · window {} days.\n\n",
                reports::local_now(),
                report.days
            ));
            body.push_str("## Demote (would move into the detail part; `--apply`)\n\n");
            if report.demote.is_empty() {
                body.push_str("none\n\n");
            } else {
                for r in &report.demote {
                    body.push_str(&format!("- ({}) {} → `{}`\n", r.date, r.text, r.pointer));
                }
                body.push('\n');
            }
            body.push_str(
                "## Promote (a detail section whose words are active but has no index row)\n\n",
            );
            if report.promote.is_empty() {
                body.push_str("none\n");
            } else {
                for p in &report.promote {
                    body.push_str(&format!("- `{}` §{}\n", p.part, p.slug));
                }
            }
            body.push_str(&format!(
                "\n**{} demote candidates · {} promote candidates**\n",
                report.demote.len(),
                report.promote.len()
            ));
            optimisemd_core::write_atomic(&path, &body)?;
            println!("candidates written to memory/logs/promotion-candidates.md");
            println!(
                "{} demote candidates · {} promote candidates",
                report.demote.len(),
                report.promote.len()
            );
        }
        return Ok(0);
    }
    // apply, then the checks gate: a failed check restores the backup and exits 1.
    let report = promoter.apply(a.days, a.dry_run)?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for line in &report.applied {
            println!("moved: {line}");
        }
        println!(
            "moved {} row(s); backup {}",
            report.applied.len(),
            report.backup.as_deref().unwrap_or("(none)")
        );
    }
    if a.dry_run {
        // A dry run wrote nothing: no gate to run, no audit line claiming an apply.
        return Ok(0);
    }
    let touched = vec![
        "MEMORY.md".to_string(),
        report
            .backup
            .clone()
            .unwrap_or_else(|| "(no backup)".to_string()),
    ];
    // The gate must RUN for the move to stand: a gate that cannot run is a failed gate, and the
    // backup is restored rather than leaving unverified changes behind.
    let check = run_check(root).unwrap_or(2);
    if check != 0 {
        eprintln!("dsh-optimisemd: the index check failed — restoring the backup");
        if let Some(b) = &report.backup {
            // Restore every file to its own relative path: MEMORY.md and the parts under
            // docs/memory-details/, exactly as the backup recorded them.
            let backup_root = Path::new(b);
            let mut restore_err = String::new();
            let mut stack = vec![(backup_root.to_path_buf(), root.to_path_buf())];
            while let Some((from, to)) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&from) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let src = entry.path();
                    let dst = to.join(entry.file_name());
                    if src.is_dir() {
                        std::fs::create_dir_all(&dst).ok();
                        stack.push((src, dst));
                    } else if let Err(e) = std::fs::copy(&src, &dst) {
                        restore_err = format!("{}: {e}", dst.display());
                    }
                }
            }
            if !restore_err.is_empty() {
                eprintln!("dsh-optimisemd: RESTORE FAILED — {restore_err}; backup kept at {b}");
            }
        }
        let _ = optimisemd_core::audit::record(root, "promote-apply", "rolled-back", &touched);
        return Ok(1);
    }
    let _ = optimisemd_core::audit::record(root, "promote-apply", "applied", &touched);
    Ok(0)
}

fn tier0(root: &Path, args: Tier0Args) -> Result<i32, Box<dyn std::error::Error>> {
    let writes = matches!(
        args.cmd,
        Tier0Cmd::Add(_)
            | Tier0Cmd::Replace(_)
            | Tier0Cmd::Remove(_)
            | Tier0Cmd::Merge(_)
            | Tier0Cmd::Import { .. }
    );
    let path = match args.cmd {
        Tier0Cmd::Add(_) | Tier0Cmd::Merge(_) | Tier0Cmd::Import { .. } => store_path_write(),
        _ => store_path_read(),
    };
    let Some(path) = path else {
        eprintln!("tier0: no store found (DSH_HOME not set and no ~/.dsh store)");
        return Ok(2);
    };
    // Read commands open the store read-only: listing or verifying must never create or touch it.
    let store = if writes {
        Store::open(&path, Default::default())?
    } else {
        Store::open_readonly(&path)?
    };
    match args.cmd {
        Tier0Cmd::List => {
            let entries = store.list_entries()?;
            let user = store.usage("user", "user-global")?;
            let agent = store.usage("agent", "workspace")?;
            println!(
                "store {} · user {}/{} · agent {}/{}",
                path.display(),
                user,
                store.cap("user"),
                agent,
                store.cap("agent")
            );
            for e in entries {
                println!(
                    "  [{}/{}] ({} chars, {}) {}",
                    e.track,
                    e.scope,
                    e.text.chars().count(),
                    e.source,
                    e.text
                );
            }
        }
        Tier0Cmd::Add(a) => {
            let tags: Vec<String> = a
                .tags
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            let id = store.add(&a.track, &a.scope, &a.text, &tags, &a.source, "", "", "")?;
            println!("added {id}");
            refresh(root);
        }
        Tier0Cmd::Replace(r) => {
            let e = store.replace(&r.match_, &r.text, "")?;
            println!("replaced {}", e.id);
            refresh(root);
        }
        Tier0Cmd::Remove(r) => {
            let e = store.remove(&r.match_, "")?;
            println!("removed {}", e.id);
            refresh(root);
        }
        Tier0Cmd::Query { text } => {
            for e in store.query(&text, 20, "")? {
                println!("  [{}/{}] {}", e.track, e.scope, e.text);
            }
        }
        Tier0Cmd::Verify { report_only } => {
            return verify(root, &store, report_only);
        }
        Tier0Cmd::History { limit } => {
            let rows = store.history(limit)?;
            if rows.is_empty() {
                println!("no audit rows");
            }
            for r in rows {
                println!(
                    "{:>4}  {}  {:<8} {}/{}  {:<9} {:<10} {}",
                    r.seq,
                    reports::local_datetime(r.ts / 1000),
                    r.action,
                    r.track,
                    r.scope,
                    r.outcome,
                    r.source,
                    shorten(&r.text, 80)
                );
            }
        }
        Tier0Cmd::Import { from } => {
            let n = store.import_from_memento(std::path::Path::new(&from))?;
            println!("imported {n} entries from {from}");
            refresh(root);
        }
        Tier0Cmd::Merge(m) => {
            let tags: Vec<String> = m
                .tags
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            // The old maintenance script backed the database up before a merge; keep that safety net.
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            // Seconds plus the process id: two merges in one second no longer share a backup name.
            let backup =
                path.with_file_name(format!("memory.db.bak-{stamp}-{}", std::process::id()));
            store.backup_to(&backup)?;
            println!("backup: {}", backup.display());
            let id = store.merge(&m.matches, &m.track, &m.scope, &m.text, &tags, "cli")?;
            println!("merged {} entr(ies) into {id}", m.matches.len());
            refresh(root);
        }
    }
    Ok(0)
}

/// The one-writer checks, ported from check-memento.sh: budgets, an agent-track entry that re-tells a MEMORY.md row, an
/// agent-track fact with no repo pointer, and the export's row count against the store.
fn verify(
    root: &Path,
    store: &Store,
    report_only: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let mut findings: Vec<String> = Vec::new();
    let user = store.usage("user", "user-global")?;
    let agent = store.usage("agent", "workspace")?;
    let user_cap = store.cap("user");
    let agent_cap = store.cap("agent");
    if user > user_cap || agent > agent_cap {
        findings.push(format!("[1] budget: user {user}/{user_cap}, agent {agent}/{agent_cap} — the store is over its cap"));
    } else if user * 100 / user_cap.max(1) >= 80 || agent * 100 / agent_cap.max(1) >= 80 {
        eprintln!("warn: budget user {user}/{user_cap}, agent {agent}/{agent_cap} (past 80%)");
    }
    let memory = match std::fs::read_to_string(root.join("MEMORY.md")) {
        Ok(m) => m,
        Err(e) => {
            findings.push(format!(
                "[0] MEMORY.md cannot be read ({e}) — this check cannot see its input and must not pass"
            ));
            String::new()
        }
    };
    let rows = optimisemd_core::promo::parse_rows(&memory);
    let stop: [&str; 12] = [
        "that", "this", "with", "from", "they", "have", "when", "what", "were", "will", "into",
        "your",
    ];
    let words = |t: &str| -> Vec<String> {
        t.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 4)
            .map(|w| w.to_lowercase())
            .filter(|w| !stop.contains(&w.as_str()))
            .collect()
    };
    let entries = store.list_entries()?;
    for e in &entries {
        if e.track != "agent" {
            continue;
        }
        let ew = words(&e.text);
        let is_pointer = e.tags.iter().any(|t| t == "pointer") && e.text.chars().count() < 250;
        if !is_pointer {
            for row in &rows {
                let shared: Vec<&String> =
                    ew.iter().filter(|w| words(&row.text).contains(w)).collect();
                if shared.len() >= 2 {
                    findings.push(format!(
                        "[2] agent/{}: entry re-tells a MEMORY.md row ({:?}) — keep only a pointer",
                        e.scope, shared
                    ));
                    break;
                }
            }
            let names_repo = ["docs/", "skills/", "plugins/"]
                .iter()
                .any(|p| e.text.contains(p));
            if !names_repo {
                findings.push(format!(
                    "[3] agent/{}: an agent-track fact with no docs/, skills/ or plugins/ pointer",
                    e.scope
                ));
            }
        }
    }
    let export =
        std::fs::read_to_string(root.join("memory/logs/memento-export.md")).unwrap_or_default();
    // One exported row per stored entry — counting entry rows, not every line that starts with a
    // dash (an entry whose text carried a newline used to flip this check).
    let exported = export.lines().filter(|l| l.starts_with("- **")).count();
    if exported != entries.len() {
        findings.push(format!("[4] the find export lists {exported} of {} stored entries — run: dsh-optimisemd export", entries.len()));
    }
    for f in &findings {
        println!("{f}");
    }
    println!(
        "tier-0: {} entries · 2 track/scope pairs · findings {}",
        entries.len(),
        findings.len()
    );
    println!("user/user-global: {user}/{user_cap} chars");
    println!("agent/workspace: {agent}/{agent_cap} chars");
    if report_only && !findings.is_empty() {
        println!(
            "(report-only: {} finding(s) — a known backlog, not a new regression)",
            findings.len()
        );
        return Ok(0);
    }
    Ok(if findings.is_empty() { 0 } else { 1 })
}

fn shorten(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

fn dsh_home() -> Option<PathBuf> {
    std::env::var("DSH_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".dsh"))
        })
}

/// The store the plugin owns; created on first write.
fn store_path_write() -> Option<PathBuf> {
    dsh_home().map(|h| h.join("dsh-optimisemd/memory.db"))
}

/// An existing store to read: the plugin's own, else the dsh-memento one it replaces.
pub(crate) fn store_path_read() -> Option<PathBuf> {
    let home = dsh_home()?;
    let new = home.join("dsh-optimisemd/memory.db");
    if new.exists() {
        return Some(new);
    }
    let old = home.join("dsh-memento/memory.db");
    if old.exists() {
        return Some(old);
    }
    None
}

/// Export the tier-0 store the way the finder reads it: a row-store document with both tracks and full text.
fn export(root: &Path) -> Result<i32, Box<dyn std::error::Error>> {
    let n = export_inner(root)?;
    if n == 0 {
        eprintln!("export: no tier-0 store found (DSH_HOME not set and no ~/.dsh store)");
        return Ok(2);
    }
    println!(
        "wrote {}: {} entries",
        root.join("memory/logs/memento-export.md").display(),
        n
    );
    Ok(0)
}

/// The finder binary: the installed one first, then the tool's build output.
fn finder(root: &Path) -> Option<PathBuf> {
    let installed = root.join("bin/dsh-belfind");
    if installed.exists() {
        return Some(installed);
    }
    let built = root.join("tools/belfind/target/release/dsh-belfind");
    built.exists().then_some(built)
}

/// Run the finder: a real executable, or — when the workspace ships the finder as a script (no
/// executable bit on unix, no handleable file type on Windows) — through a shell that can run it.
fn run_finder(
    bin: &Path,
    root: &Path,
    args: &[&str],
) -> Result<std::process::ExitStatus, Box<dyn std::error::Error>> {
    let mut direct = Command::new(bin);
    direct.args(args).current_dir(root);
    match direct.status() {
        Ok(st) => Ok(st),
        Err(_) => Ok(optimisemd_core::util::run_script_status(bin, root, args)?),
    }
}

fn index(root: &Path) -> Result<i32, Box<dyn std::error::Error>> {
    let Some(bin) = finder(root) else {
        eprintln!("index: no dsh-belfind binary: (cd tools/belfind && cargo build --release)");
        return Ok(2);
    };
    let status = run_finder(&bin, root, &["index"])?;
    Ok(status.code().unwrap_or(1))
}

fn find(root: &Path, reindex: bool, query: Vec<String>) -> Result<i32, Box<dyn std::error::Error>> {
    let Some(bin) = finder(root) else {
        eprintln!("find: no dsh-belfind binary: (cd tools/belfind && cargo build --release)");
        return Ok(1);
    };
    let mut reindex = reindex;
    let mut query = query;
    // `find index` (the whole query) is the reindex shorthand; a QUESTION beginning with the word
    // "index" is a question and reaches the finder unchanged.
    if query.len() == 1 && query[0] == "index" {
        reindex = true;
        query.clear();
    }
    if !reindex && query.is_empty() {
        let mut cmd = Cli::command();
        if let Some(sub) = cmd.find_subcommand_mut("find") {
            sub.print_help()?;
        }
        return Ok(0);
    }
    // A fresh checkout or a cleaned corpus must not silently answer from less: rebuild what the finder reads.
    let pieces = [
        "memory/logs/session-digest.md",
        "memory/logs/memento-export.md",
        "memory/logs/find-index.json",
    ];
    if pieces.iter().any(|p| !root.join(p).exists()) {
        if let Err(e) = reports::session_digest(root) {
            eprintln!("warning: session digest could not be regenerated — historical search may be incomplete: {e}");
        }
        let _ = export_inner(root);
        let _ = run_finder(&bin, root, &["index"]);
    }
    let mut args: Vec<&str> = Vec::new();
    if reindex {
        args.push("index");
    } else {
        args.push("find");
    }
    args.extend(query.iter().map(|q| q.as_str()));
    let status = run_finder(&bin, root, &args)?;
    Ok(status.code().unwrap_or(1))
}

/// A store write is only useful once the finder can see it: refresh the export and the index the way the nightly does.
fn refresh(root: &Path) {
    let _ = export_inner(root);
    if let Some(bin) = finder(root) {
        // The same direct-then-shell rule as `find`: a script finder cannot be spawned directly on
        // Windows (error 193), and swallowing that error left the index stale without a word. A
        // finder that starts and then fails is the same outcome for the reader — a stale index —
        // so a non-zero exit is reported too, not only a spawn error (v0.4.12).
        match run_finder(&bin, root, &["index"]) {
            Ok(status) if status.success() => {}
            Ok(status) => eprintln!(
                "warning: the search index was not refreshed (the finder exited with {status})"
            ),
            Err(e) => eprintln!("warning: the search index was not refreshed ({e})"),
        }
    }
}

fn export_inner(root: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    let Some(src) = store_path_read() else {
        return Ok(0);
    };
    // A read-only handle reads a CONSISTENT snapshot through SQLite; the old copy of database,
    // -wal and -shm files at three instants could tear and left working files in the temp dir.
    let store = Store::open_readonly(&src)?;
    let entries = store.list_entries()?;
    let out = root.join("memory/logs/memento-export.md");
    std::fs::create_dir_all(out.parent().unwrap_or(Path::new(".")))?;
    let mut body = String::from(
        "# The plugin memory store (tier 0), exported for the index\n\nGenerated by `dsh-optimisemd export` from a read-only copy of the store, with the FULL text of every entry. Both tracks are included.\n\n",
    );
    for e in &entries {
        body.push_str(&format!(
            "- **{}/{}** {:?} ({}): {}\n",
            e.track,
            e.scope,
            e.tags,
            e.source,
            e.text.trim()
        ));
    }
    optimisemd_core::write_atomic(&out, &body)?;
    Ok(entries.len())
}

fn run_check(root: &Path) -> Result<i32, Box<dyn std::error::Error>> {
    let script = root.join("scripts/check-memory-index.sh");
    if !script.exists() {
        eprintln!(
            "check: no scripts/check-memory-index.sh under {}",
            root.display()
        );
        return Ok(2);
    }
    // Direct execution: a shim or the binary; `bash` cannot run the binary form. Hosts that cannot
    // execute a .sh at all fall back through the shared runner (bash, then sh).
    let out = optimisemd_core::util::run_script(&script, root)?;
    Ok(out.status.code().unwrap_or(1))
}
