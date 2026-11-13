//! dsh-optimisemd — the command line the workspace's nightlies call. One binary owns promotion, the checks gate, the
//! tier-0 export and the finder refresh, so the shell scripts become either thin aliases or disappear.

use clap::{Args, Parser, Subcommand};
use optimisemd_core::{Promoter, Store};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser)]
#[command(
    name = "dsh-optimisemd",
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
    /// Run the workspace's index check (the same bash checker every commit runs).
    Check,
    /// Write memory/logs/memento-export.md from a read-only copy of the tier-0 store.
    Export,
    /// Rebuild the ranked finder index (delegates to the workspace's finder).
    Index,
}

#[derive(Args)]
struct PromoteArgs {
    #[arg(long, default_value_t = 180)]
    days: u32,
    #[arg(long)]
    apply: bool,
    #[arg(long)]
    json: bool,
}

fn main() {
    let cli = Cli::parse();
    let root = cli.root;
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
        Cmd::Check => check(root),
        Cmd::Export => export(root),
        Cmd::Index => index(root),
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
        }
        return Ok(0);
    }
    // apply, then the checks gate: a failed check restores the backup and exits 1.
    let report = promoter.apply(a.days, false)?;
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
    let check = run_check(root)?;
    if check != 0 {
        eprintln!("dsh-optimisemd: the index check failed — restoring the backup");
        if let Some(b) = &report.backup {
            let _ = std::fs::copy(Path::new(b).join("MEMORY.md"), root.join("MEMORY.md"));
            for entry in std::fs::read_dir(b).into_iter().flatten().flatten() {
                let p = entry.path();
                if p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .as_deref()
                    == Some("MEMORY.md")
                {
                    continue;
                }
                if let Some(name) = p.file_name() {
                    let _ = std::fs::copy(&p, root.join("docs/memory-details").join(name));
                }
            }
        }
        return Ok(1);
    }
    Ok(0)
}

fn check(root: &Path) -> Result<i32, Box<dyn std::error::Error>> {
    let code = run_check(root)?;
    if code == 0 {
        println!("check: ok");
    }
    Ok(code)
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
    let status = Command::new("bash")
        .arg(&script)
        .current_dir(root)
        .status()?;
    Ok(status.code().unwrap_or(1))
}

fn store_path(root: &Path) -> Option<PathBuf> {
    let home = std::env::var("DSH_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".dsh"))
        })?;
    let new = home.join("dsh-optimisemd/memory.db");
    let old = home.join("dsh-memento/memory.db");
    if new.exists() {
        Some(new)
    } else if old.exists() {
        Some(old)
    } else {
        let _ = root;
        None
    }
}

/// Export the tier-0 store the way the finder reads it: a row-store document with both tracks and full text.
fn export(root: &Path) -> Result<i32, Box<dyn std::error::Error>> {
    let Some(src) = store_path(root) else {
        eprintln!("export: no tier-0 store found (DSH_HOME not set and no ~/.dsh store)");
        return Ok(2);
    };
    let tmp = std::env::temp_dir().join(format!("optimisemd-export-{}.db", std::process::id()));
    std::fs::copy(&src, &tmp)?;
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{}", src.display(), suffix));
        if side.exists() {
            let _ = std::fs::copy(&side, PathBuf::from(format!("{}{}", tmp.display(), suffix)));
        }
    }
    let store = Store::open(&tmp, Default::default())?;
    let entries = store.list_entries()?;
    let out = root.join("memory/logs/memento-export.md");
    std::fs::create_dir_all(out.parent().unwrap_or(Path::new(".")))?;
    let mut body = String::from("# The plugin memory store (tier 0), exported for the index\n\nGenerated by `dsh-optimisemd export` from a read-only copy of the store, with the FULL text of every entry. Both tracks are included.\n\n");
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
    std::fs::write(&out, body)?;
    println!("wrote {}: {} entries", out.display(), entries.len());
    let _ = std::fs::remove_file(&tmp);
    Ok(0)
}

fn index(root: &Path) -> Result<i32, Box<dyn std::error::Error>> {
    let script = root.join("scripts/find.sh");
    if !script.exists() {
        eprintln!("index: no scripts/find.sh under {}", root.display());
        return Ok(2);
    }
    let status = Command::new("bash")
        .arg(&script)
        .arg("--reindex")
        .current_dir(root)
        .status()?;
    Ok(status.code().unwrap_or(1))
}
