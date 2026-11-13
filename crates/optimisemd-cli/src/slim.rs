//! Markdown slimming, measured — the P7 port of `scripts/md-slimming-report.sh`.
//!
//! Budgets are parsed out of the workspace guard so the report and the guard cannot disagree. Sizes come from `git show
//! <baseline>:<file>` and the working tree; the report is written to `docs/md-slimming-report.md` and the summary line is
//! printed. Exit 1 when a budgeted file is over budget.

use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Row {
    pub file: String,
    pub old: Option<u64>,
    pub now: u64,
    pub budget: Option<(u64, u64)>, // (lines, chars)
}

pub struct SlimArgs<'a> {
    pub baseline: Option<&'a str>,
}

fn fmt(n: i64) -> String {
    let neg = n < 0;
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

fn git_show_size(root: &Path, rev: &str, path: &str) -> Option<u64> {
    let out = Command::new("git")
        .arg("show")
        .arg(format!("{rev}:{path}"))
        .current_dir(root)
        .output()
        .ok()?;
    if out.status.success() {
        Some(out.stdout.len() as u64)
    } else {
        None
    }
}

type Budget = (String, u64, u64);

fn read_budgets(guard: &str) -> Result<Vec<Budget>, Box<dyn Error>> {
    let start = guard
        .find("BUDGETS_BLOCK=$(cat <<'BUDGETS'\n")
        .ok_or("no BUDGETS_BLOCK found in scripts/check-context-files.sh")?
        + "BUDGETS_BLOCK=$(cat <<'BUDGETS'\n".len();
    let body = &guard[start..];
    let end = body.find("\nBUDGETS").ok_or("unterminated BUDGETS block")?;
    let mut out = Vec::new();
    for line in body[..end].lines() {
        let fields: Vec<&str> = line
            .split(['\'', ',', '(', ')'])
            .filter(|s| !s.trim().is_empty())
            .collect();
        // ('AGENTS.md', 150, 20000) → [AGENTS.md, 150, 20000]
        if fields.len() == 3 {
            if let (Ok(lines), Ok(chars)) = (
                fields[1].trim().parse::<u64>(),
                fields[2].trim().parse::<u64>(),
            ) {
                out.push((fields[0].trim().to_string(), lines, chars));
            }
        }
    }
    Ok(out)
}

fn line_count(path: &Path) -> Result<u64, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let nl = bytes.iter().filter(|b| **b == b'\n').count() as u64;
    let trailing = if bytes.last() == Some(&b'\n') { 0 } else { 1 };
    Ok(nl + trailing)
}

pub fn run(root: &Path, args: SlimArgs) -> Result<i32, Box<dyn Error>> {
    let baseline = match args.baseline {
        Some(b) if !b.is_empty() => b.to_string(),
        _ => {
            let out = Command::new("git")
                .args(["rev-list", "--max-parents=0", "HEAD"])
                .current_dir(root)
                .output()?;
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        }
    };
    let guard = fs::read_to_string(root.join("scripts/check-context-files.sh"))?;
    let budgets_raw = read_budgets(&guard)?;
    let mut budgets: HashMap<String, (u64, u64)> = HashMap::new();
    for (f, l, c) in &budgets_raw {
        budgets.insert(f.clone(), (*l, *c));
    }
    let mut files: Vec<String> = vec!["AGENTS.md".into(), "MEMORY.md".into()];
    for (f, _, _) in &budgets_raw {
        if f != "AGENTS.md" && f != "MEMORY.md" && !files.contains(f) {
            files.push(f.clone());
        }
    }
    for extra in [
        "docs/lesson-details.md",
        "docs/workflows.md",
        "docs/session-recovery.md",
        "docs/RULES.md",
    ] {
        if root.join(extra).exists() && !files.iter().any(|f| f == extra) {
            files.push(extra.into());
        }
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut over: Vec<String> = Vec::new();
    for f in files {
        let path = root.join(&f);
        if !path.exists() {
            continue;
        }
        let now = fs::metadata(&path)?.len();
        let old = git_show_size(root, &baseline, &f);
        let lines = line_count(&path)?;
        let budget = budgets.get(&f).copied();
        if let Some((bl, bc)) = budget {
            if now > bc || lines > bl {
                over.push(f.clone());
            }
        }
        rows.push(Row {
            file: f,
            old,
            now,
            budget,
        });
    }
    let mut heads: Vec<&Row> = rows
        .iter()
        .filter(|r| r.file == "AGENTS.md" || r.file == "MEMORY.md")
        .collect();
    heads.sort_by(|a, b| a.file.cmp(&b.file));
    let hnow: u64 = heads.iter().map(|r| r.now).sum();
    let hold: u64 = heads.iter().filter_map(|r| r.old).sum();
    let saved = hold as i64 - hnow as i64;
    let pct = if hold > 0 {
        saved * 100 / hold as i64
    } else {
        0
    };
    let mut split: i64 = 0;
    let mut inplace: i64 = 0;
    let mut growth: i64 = 0;
    for r in &rows {
        match r.old {
            None => growth += r.now as i64,
            Some(old) => {
                let d = old as i64 - r.now as i64;
                if (r.file == "MEMORY.md" || r.file == "docs/lesson-details.md") && old > r.now * 5
                {
                    split += d;
                } else if d >= 0 {
                    inplace += d;
                } else {
                    growth += -d;
                }
            }
        }
    }
    let line = format!(
        "always-read surface: {} → {} chars · {} chars saved ({}% smaller) vs {} · {} files measured · {} over budget",
        fmt(hold as i64), fmt(hnow as i64), fmt(saved), pct, &baseline[..baseline.len().min(8)], rows.len(), over.len()
    );
    let stamp = Command::new("date")
        .arg("+%Y-%m-%d %H:%M")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let mut out = String::new();
    out.push_str("# Markdown slimming — measured, not claimed\n\n");
    out.push_str(&format!(
        "Generated by `bin/dsh-optimisemd slim` at {stamp}, baseline commit `{}`. Budgets are parsed out of `scripts/check-context-files.sh`, so this table and the guard cannot disagree. A negative change is slimming; a positive one is growth — both are facts worth seeing.\n\n",
        &baseline[..baseline.len().min(8)]
    ));
    out.push_str("| File | Baseline | Now | Change | Budget (chars) | % of budget |\n|---|---|---|---|---|---|\n");
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by(|a, b| a.file.cmp(&b.file));
    for r in &sorted {
        let o = r
            .old
            .map(|v| fmt(v as i64))
            .unwrap_or_else(|| "absent".into());
        let d = if let Some(v) = r.old {
            let delta = r.now as i64 - v as i64;
            if delta >= 0 {
                format!("+{}", fmt(delta))
            } else {
                fmt(delta)
            }
        } else {
            "—".into()
        };
        let btxt = r
            .budget
            .map(|(_, c)| fmt(c as i64))
            .unwrap_or_else(|| "—".into());
        let ptxt = r
            .budget
            .map(|(_, c)| format!("{}%", r.now * 100 / c.max(1)))
            .unwrap_or_else(|| "—".into());
        let mark = if over.contains(&r.file) {
            " **over**"
        } else {
            ""
        };
        out.push_str(&format!(
            "| `{}` | {o} | {} | {d} | {btxt} | {ptxt}{mark} |\n",
            r.file,
            fmt(r.now as i64)
        ));
    }
    out.push('\n');
    out.push_str(&format!("**{line}**\n\n"));
    out.push_str(&format!(
        "Across all {} measured files: **{} chars moved into parts** (an index split — `MEMORY.md` and `lesson-details.md` became indexes), **{} cut in place**, **{} grown**. Only the middle figure is slimming; the first is a restructure, the last is cost — and the headline above counts only the always-read pair, so the three do not add up to it by design.\n",
        rows.len(), fmt(split), fmt(inplace), fmt(growth)
    ));
    fs::create_dir_all(root.join("docs"))?;
    fs::write(root.join("docs/md-slimming-report.md"), out)?;
    println!("{line}");
    if !over.is_empty() {
        println!("over budget: {}", over.join(", "));
        return Ok(1);
    }
    Ok(0)
}
