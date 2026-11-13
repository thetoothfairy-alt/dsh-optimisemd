//! The MEMORY.md index check, ported from `scripts/check-memory-index.sh`. Nine rules, one summary line; the bash script is now
//! a wrapper over this so the commit hook, the save gate and a hand run all share one implementation.

use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::Path;

const MAX: usize = 200;

const STOPWORDS: &[&str] = &[
    "should", "because", "before", "anything", "without", "every", "never", "through", "always", "little",
    "until", "after", "there", "these", "those", "which", "while", "where", "their", "about", "other",
    "still", "being", "would", "could", "reports", "report", "system", "restart",
];

fn cells(line: &str) -> Vec<String> {
    line.trim().trim_matches('|').split(" | ").map(|c| c.trim().to_string()).collect()
}

fn words_of(fact: &str) -> HashSet<String> {
    let re = Regex::new(r"[A-Za-z]{6,}").unwrap();
    re.find_iter(fact).map(|m| m.as_str().to_lowercase())
        .filter(|w| !STOPWORDS.contains(&w.as_str())).collect()
}

pub fn run(root: &Path, targets: Vec<String>) -> Result<i32, Box<dyn Error>> {
    let mut targets = targets;
    if targets.is_empty() {
        targets.push("MEMORY.md".into());
        if root.join("docs/lesson-details.md").exists() {
            targets.push("docs/lesson-details.md".into());
        }
    }
    for t in &targets {
        if !root.join(t).exists() {
            eprintln!("no such file: {t}");
            return Ok(2);
        }
    }
    let row_re = Regex::new(r"^- \*\*.+?\*\*.*? — `([^`]+\.md)`\s*$")?;
    let slug_re = Regex::new(r"^`§[a-z0-9-]+`\s*$")?;
    let slug_ok = Regex::new(r"^[a-z0-9-]+$")?;
    let mut findings: Vec<String> = Vec::new();
    let mut fact_rows = 0usize;
    let mut detail_rows = 0usize;
    let mut pointed: HashSet<String> = HashSet::new();

    for target in &targets {
        let text = fs::read_to_string(root.join(target)).unwrap_or_default();
        let mut section: Option<String> = None;
        for (i, line) in text.lines().enumerate() {
            let i = i + 1;
            if line.starts_with("## ") {
                section = Some(line.trim().to_string());
            }
            if let Some(caps) = row_re.captures(line) {
                fact_rows += 1;
                if section.as_deref().map_or(true, |s| s.starts_with("## Detail")) {
                    findings.push(format!("{target}:{i}: a fact row outside a subject section — put it under a `##` heading, above the detail table"));
                }
                let path = caps[1].to_string();
                let text_before = line.split(" — `").next().unwrap_or("").to_string();
                if text_before.chars().count() > MAX {
                    findings.push(format!("{target}:{i}: row text is {} chars, cap {MAX} — split the fact or move detail out", text_before.chars().count()));
                }
                if !root.join(&path).exists() {
                    findings.push(format!("{target}:{i}: points at {path}, which does not exist"));
                } else {
                    pointed.insert(path.clone());
                    let family = path.starts_with("docs/memory-details/") || path.starts_with("docs/runbooks/")
                        || path.starts_with("skills/") || path == "docs/hard-won.md";
                    if !family {
                        findings.push(format!("{target}:{i}: a fact row hops to {path} — point it at a docs/memory-details/ part, its runbook, a skill, or docs/hard-won.md (docs/runbooks/memory.md)"));
                    }
                }
                continue;
            }
            if line.starts_with("- **") {
                findings.push(format!("{target}:{i}: a `- **…**` row that does not end in a backticked `<file>.md` pointer — this check cannot see it (length, broken link, duplicate subject); end the row with the file that carries the mechanism"));
                continue;
            }
            if line.starts_with("| §") {
                let cs = cells(line);
                if cs.len() < 2 { continue; }
                let slug = cs[0].trim_start_matches('§').to_string();
                let last = cs.last().unwrap().trim_matches('`').to_string();
                if !last.ends_with(".md") { continue; }
                detail_rows += 1;
                if slug.is_empty() || !slug_ok.is_match(&slug) {
                    findings.push(format!("{target}:{i}: detail row's first cell is not a §slug"));
                } else if !root.join(&last).exists() {
                    findings.push(format!("{target}:{i}: §{slug} points at {last}, which does not exist"));
                } else if !fs::read_to_string(root.join(&last)).unwrap_or_default().contains(&format!("§{slug}")) {
                    findings.push(format!("{target}:{i}: §{slug} is not in {last}"));
                } else {
                    pointed.insert(last);
                }
            }
        }
    }

    // [7] one subject, one row: a shared pointer and two shared distinctive words are the same subject written twice.
    let mut seen: HashMap<String, Vec<(usize, HashSet<String>)>> = HashMap::new();
    for target in &targets {
        let text = fs::read_to_string(root.join(target)).unwrap_or_default();
        for (i, line) in text.lines().enumerate() {
            let i = i + 1;
            let Some(caps) = row_re.captures(line) else { continue };
            let mut fact = line.strip_prefix("- **").unwrap_or(line).to_string();
            if fact.ends_with("**") { fact.truncate(fact.len() - 2); }
            let fact = fact.split(" — `").next().unwrap_or("").to_string();
            let words = words_of(&fact);
            let key = caps[1].to_string();
            if let Some(prev) = seen.get(&key) {
                for (oj, owords) in prev {
                    let mut shared: Vec<&String> = words.intersection(owords).collect();
                    if shared.len() >= 2 {
                        shared.sort();
                        findings.push(format!("{target}:{i}: same subject as line {oj} ({}) — one row per subject; edit the existing row or supersede it (docs/runbooks/memory.md)", shared.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
                    }
                }
            }
            seen.entry(key).or_default().push((i, words));
        }
    }

    // [5] and orphans: every part is pointed at, and a §slug label under a carrying heading is a duplicate.
    let mut parts: Vec<String> = Vec::new();
    for dir in ["docs/memory-details", "docs/lessons"] {
        if let Ok(rd) = fs::read_dir(root.join(dir)) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.ends_with(".md") { parts.push(format!("{dir}/{n}")); }
            }
        }
    }
    parts.sort();
    for part in &parts {
        if !pointed.contains(part) {
            findings.push(format!("{part}: no index row points at this part (orphan)"));
        }
        let text = fs::read_to_string(root.join(part)).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if slug_re.is_match(line) && (i.saturating_sub(3)..i).any(|j| lines[j].starts_with("### ")) {
                findings.push(format!("{part}:{}: a `§slug` label sits under a heading that already carries the address", i + 1));
            }
        }
    }

    for f in &findings {
        println!("{f}");
    }
    println!("indexes {} · fact rows {} (cap {}) · detail rows {} · findings {}", targets.len(), fact_rows, MAX, detail_rows, findings.len());
    Ok(if findings.is_empty() { 0 } else { 1 })
}
