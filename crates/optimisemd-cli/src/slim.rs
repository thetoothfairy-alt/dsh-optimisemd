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
        Some(String::from_utf8_lossy(&out.stdout).chars().count() as u64)
    } else {
        None
    }
}

/// Shorten a revision for display — by CHARACTERS. A multi-byte baseline once panicked the report
/// through a byte slice.
fn short(rev: &str) -> String {
    rev.chars().take(8).collect()
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

/// The budget of one file from the one budget home: an explicit row wins over its family row
/// ('dir/*.md'), the same precedence the context checker enforces, so the writer and the gate can
/// never disagree about when a part is full.
pub(crate) fn budget_for(guard: &str, rel: &str) -> Option<(u64, u64)> {
    let rows = read_budgets(guard).ok()?;
    if let Some((_, l, c)) = rows.iter().find(|(p, _, _)| p == rel) {
        return Some((*l, *c));
    }
    let name = rel.rsplit_once('/').map(|(_, n)| n).unwrap_or(rel);
    for (p, l, c) in &rows {
        if let Some((dir, file_pat)) = p.rsplit_once('/') {
            let (pre, suf) = file_pat.split_once('*').unwrap_or(("", ""));
            if rel.starts_with(&format!("{dir}/"))
                && name.starts_with(pre)
                && name.ends_with(suf)
            {
                return Some((*l, *c));
            }
        }
    }
    None
}

/// Would appending `add` to `before` cross the budget? Counted exactly the way the context checker
/// counts (split on '\n', lines = entries-1, chars = Σ(len+1)), so a part the writer rotates is
/// never one the gate would have refused.
pub(crate) fn crosses_budget(before: &str, add: &str, budget: Option<(u64, u64)>) -> bool {
    let Some((maxl, maxc)) = budget else {
        return false;
    };
    let mut t = before.to_string();
    if !t.ends_with('\n') {
        t.push('\n');
    }
    t.push_str(add);
    if !t.ends_with('\n') {
        t.push('\n');
    }
    let ls: Vec<&str> = t.split('\n').collect();
    let n = ls.len().saturating_sub(1) as u64;
    let c: u64 = ls.iter().map(|l| l.chars().count() as u64 + 1).sum();
    n > maxl || c > maxc
}

fn char_count(path: &Path) -> Result<u64, Box<dyn Error>> {
    Ok(fs::read_to_string(path)?.chars().count() as u64)
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
        let now = char_count(&path)?;
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
        fmt(hold as i64), fmt(hnow as i64), fmt(saved), pct, short(&baseline), rows.len(), over.len()
    );
    let stamp = optimisemd_core::local_now();
    let mut out = String::new();
    out.push_str("# Markdown slimming — measured, not claimed\n\n");
    out.push_str(&format!(
        "Generated by `bin/dsh-optimisemd slim` at {stamp}, baseline commit `{}`. Budgets are parsed out of `scripts/check-context-files.sh`, so this table and the guard cannot disagree. A negative change is slimming; a positive one is growth — both are facts worth seeing.\n\n",
        short(&baseline)
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
    optimisemd_core::write_atomic(&root.join("docs/md-slimming-report.md"), &out)?;
    println!("{line}");
    println!(
        "cut in place {} chars · moved into parts {} · grown {} chars",
        fmt(inplace),
        fmt(split),
        fmt(growth)
    );
    if !over.is_empty() {
        println!("over budget: {}", over.join(", "));
        return Ok(1);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_file_is_measured_in_characters_not_bytes() {
        let dir = std::env::temp_dir().join(format!("optimisemd-slim-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.md");
        fs::write(&p, "a—b\n").unwrap();
        assert_eq!(char_count(&p).unwrap(), 4);
    }

    #[test]
    fn a_multibyte_baseline_shortens_by_characters() {
        // The old byte slice panicked on the first multi-byte character of a branch name.
        assert_eq!(short("äääääääää"), "ääääääää");
        assert_eq!(short("abc"), "abc");
    }
}
