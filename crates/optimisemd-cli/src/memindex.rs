//! The MEMORY.md index check, ported from `scripts/check-memory-index.sh`. Nine rules, one summary line; the bash script is now
//! a wrapper over this so the commit hook, the save gate and a hand run all share one implementation.

use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::Path;

const MAX: usize = 200;

const STOPWORDS: &[&str] = &[
    "should", "because", "before", "anything", "without", "every", "never", "through", "always",
    "little", "until", "after", "there", "these", "those", "which", "while", "where", "their",
    "about", "other", "still", "being", "would", "could", "reports", "report", "system", "restart",
];

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split(" | ")
        .map(|c| c.trim().to_string())
        .collect()
}

fn words_of(fact: &str) -> HashSet<String> {
    let re = Regex::new(r"[A-Za-z]{6,}").unwrap();
    re.find_iter(fact)
        .map(|m| m.as_str().to_lowercase())
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .collect()
}

/// The fact text and the date of a parseable row: `- **<text>** (<date>) — \`<file>.md\``.
pub(crate) fn fact_parts(line: &str) -> Option<(String, Option<String>)> {
    let rest = line.strip_prefix("- **")?;
    let mid = rest.find("** (")?;
    let text = rest[..mid].to_string();
    let after = &rest[mid + 4..];
    let close = after.find(')')?;
    let date = after[..close].to_string();
    Some((text, Some(date)))
}

/// The belief files and the diary a credential must never reach. The scan is deliberately narrow:
/// it fires on credential-shaped values only, so prose, commit hashes and `op://` fetch references
/// pass untouched.
fn secret_corpus(root: &Path) -> Vec<String> {
    let mut files: Vec<String> = [
        "MEMORY.md",
        "USER.md",
        "AGENTS.md",
        "CLAUDE.md",
        "docs/RULES.md",
        "docs/hard-won.md",
        "docs/hard-won-details.md",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for dir in [
        "docs/memory-details",
        "docs/runbooks",
        "docs/lessons",
        "docs/rules-evidence",
        "docs/hard-won",
    ] {
        if let Ok(rd) = fs::read_dir(root.join(dir)) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.ends_with(".md") {
                    files.push(format!("{dir}/{n}"));
                }
            }
        }
    }
    if let Ok(rd) = fs::read_dir(root.join("skills")) {
        for e in rd.flatten() {
            let skill = root.join("skills").join(e.file_name()).join("SKILL.md");
            if skill.is_file() {
                files.push(format!(
                    "skills/{}/SKILL.md",
                    e.file_name().to_string_lossy()
                ));
            }
        }
    }
    // The diary too: a secret pasted into today's line is still a secret in the repo. Only dated
    // files — logs/ and baselines/ stay out of the scan (docs/memory-process.md §2).
    if let Ok(rd) = fs::read_dir(root.join("memory")) {
        let day_re = Regex::new(r"^\d{4}-\d{2}-\d{2}\.md$").unwrap();
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if day_re.is_match(&n) {
                files.push(format!("memory/{n}"));
            }
        }
    }
    files.sort();
    files
}

/// A credential-looking value in a file is a finding naming the file, the line and the key it sits
/// under — never the value itself, because a secret printed is a secret leaked again.
fn secret_findings(root: &Path) -> Vec<String> {
    // Declared, not hidden (the pattern of scripts/check-md-contracts.sh): a `file:line` address in
    // docs/secret-scan-baseline.txt (tracked, so the declaration travels with the repo) is a
    // historical value recorded before this scan existed, at a line the append-only diary cannot be
    // edited at. The credential behind it is rotated; the declaration keeps the gate green for new
    // work without hiding the past.
    let mut allowed: HashSet<String> = HashSet::new();
    if let Ok(text) = fs::read_to_string(root.join("docs/secret-scan-baseline.txt")) {
        for l in text.lines() {
            let l = l.trim();
            if !l.is_empty() && !l.starts_with('#') {
                allowed.insert(l.to_string());
            }
        }
    }
    let key_re = Regex::new(
        r#"(?:^|[^A-Za-z])(?i)(api[_-]?key|apikey|secret|passwd|password|token)["']?\s*[:=]\s*["']?([A-Za-z0-9+/_\-.=]{16,})"#,
    )
    .unwrap();
    let sk_re = Regex::new(r"(?:^|[^A-Za-z0-9_-])(sk-[A-Za-z0-9]{20,})").unwrap();
    let pem_re = Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY").unwrap();
    let placeholder = |v: &str| -> bool {
        let v = v.to_lowercase();
        v.starts_with('<')
            || v.starts_with("${")
            || v.starts_with("$(")
            || v.starts_with("{{")
            || v.starts_with("op://")
            || v.starts_with("0x")
            || v.contains("example")
            || v.contains("xxxx")
            || v.contains("redacted")
            || v.contains("placeholder")
            || matches!(
                v.as_str(),
                "local" | "none" | "null" | "dummy" | "changeme" | "notasecret"
            )
    };
    let mut out = Vec::new();
    for path in secret_corpus(root) {
        let Ok(text) = fs::read_to_string(root.join(&path)) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let i = i + 1;
            if allowed.contains(&format!("{path}:{i}")) {
                continue;
            }
            if pem_re.is_match(line) {
                out.push(format!(
                    "{path}:{i}: a private key block — secrets are fetched from 1Password (docs/credentials.md), never stored"
                ));
                continue;
            }
            if sk_re.is_match(line) {
                out.push(format!(
                    "{path}:{i}: an API-key-shaped token — secrets are fetched from 1Password (docs/credentials.md), never stored"
                ));
                continue;
            }
            for caps in key_re.captures_iter(line) {
                if !placeholder(&caps[2]) {
                    out.push(format!(
                        "{path}:{i}: a credential-looking value assigned to `{}` — secrets are fetched from 1Password (docs/credentials.md), never stored",
                        &caps[1]
                    ));
                }
            }
        }
    }
    out
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
    let mut rows_per_target: Vec<(String, usize)> = Vec::new();
    let mut detail_rows = 0usize;
    let mut pointed: HashSet<String> = HashSet::new();
    let mut target_texts: HashMap<String, String> = HashMap::new();

    for target in &targets {
        // A check that cannot read its input must fail, not claim zero findings: an unreadable or
        // non-UTF8 index is a finding, exactly as the regenerated reports treat theirs.
        let text = match fs::read_to_string(root.join(target)) {
            Ok(t) => t,
            Err(e) => {
                findings.push(format!(
                    "{target}: cannot be read ({e}) — this check cannot see its input and must not pass"
                ));
                continue;
            }
        };
        let mut section: Option<String> = None;
        let mut target_rows = 0usize;
        for (i, line) in text.lines().enumerate() {
            let i = i + 1;
            if line.starts_with("## ") {
                section = Some(line.trim().to_string());
            }
            if let Some(caps) = row_re.captures(line) {
                fact_rows += 1;
                target_rows += 1;
                if section
                    .as_deref()
                    .is_none_or(|s| s.starts_with("## Detail"))
                {
                    findings.push(format!("{target}:{i}: a fact row outside a subject section — put it under a `##` heading, above the detail table"));
                }
                let path = caps[1].to_string();
                let parsed = fact_parts(line);
                let fact_text = parsed.as_ref().map(|(f, _)| f.clone()).unwrap_or_default();
                // The cap counts the FACT text; the date and the pointer sit outside it, as documented.
                match parsed {
                    Some((fact, Some(date))) => {
                        if fact.chars().count() > MAX {
                            findings.push(format!("{target}:{i}: row text is {} chars, cap {MAX} — split the fact or move detail out", fact.chars().count()));
                        }
                        if !optimisemd_core::repo::valid_date(&date) {
                            findings.push(format!(
                                "{target}:{i}: row carries ({date}), which is not a real YYYY-MM-DD date"
                            ));
                        }
                    }
                    _ => {
                        findings.push(format!(
                            "{target}:{i}: a row without its `(YYYY-MM-DD)` date — the date answers how long the fact has stood (docs/runbooks/memory.md)"
                        ));
                    }
                }
                if !root.join(&path).exists() {
                    findings.push(format!(
                        "{target}:{i}: points at {path}, which does not exist"
                    ));
                } else {
                    pointed.insert(path.clone());
                    let family = path.starts_with("docs/memory-details/")
                        || path.starts_with("docs/runbooks/")
                        || path.starts_with("skills/")
                        || path == "docs/hard-won.md"
                        || path.starts_with("docs/hard-won/");
                    if !family {
                        findings.push(format!("{target}:{i}: a fact row hops to {path} — point it at a docs/memory-details/ part, its runbook, a skill, or docs/hard-won.md (docs/runbooks/memory.md)"));
                    }
                    // A row can be perfectly shaped and still describe something that changed: no
                    // other rule reads the file the row points at. Two or more distinctive words and
                    // not one of them in the target text is the staleness signal (calibrated
                    // 2026-11-22 against the real index).
                    let words = words_of(&fact_text);
                    if words.len() >= 2 {
                        let tgt = target_texts.entry(path.clone()).or_insert_with(|| {
                            fs::read_to_string(root.join(&path))
                                .unwrap_or_default()
                                .to_lowercase()
                        });
                        if !words.iter().any(|w| tgt.contains(w.as_str())) {
                            findings.push(format!("{target}:{i}: no distinctive word of the row appears in {path} — the row may describe something that has changed; verify the fact still holds, then refresh the row or its pointer"));
                        }
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
                if cs.len() < 2 {
                    continue;
                }
                let slug = cs[0].trim_start_matches('§').to_string();
                let last = cs.last().unwrap().trim_matches('`').to_string();
                if !last.ends_with(".md") {
                    continue;
                }
                detail_rows += 1;
                if slug.is_empty() || !slug_ok.is_match(&slug) {
                    findings.push(format!(
                        "{target}:{i}: detail row's first cell is not a §slug"
                    ));
                } else if !root.join(&last).exists() {
                    findings.push(format!(
                        "{target}:{i}: §{slug} points at {last}, which does not exist"
                    ));
                } else if !fs::read_to_string(root.join(&last))
                    .unwrap_or_default()
                    .contains(&format!("§{slug}"))
                {
                    findings.push(format!("{target}:{i}: §{slug} is not in {last}"));
                } else {
                    pointed.insert(last);
                }
            }
        }
        rows_per_target.push((target.clone(), target_rows));
    }

    // The documented index budget: MEMORY.md carries at most 200 rows. The count is enforced, not
    // merely printed — retire quiet rows with `bin/dsh-optimisemd promote`.
    for (target, n) in &rows_per_target {
        if target == "MEMORY.md" && *n > 200 {
            findings.push(format!(
                "{target}: holds {n} rows, the cap is 200 — retire quiet rows (bin/dsh-optimisemd promote)"
            ));
        }
    }

    // [7] one subject, one row. Same pointer + two shared distinctive words (as before), or FOUR
    // shared distinctive words across DIFFERENT pointers — calibrated 2026-11-16 against the real
    // index, where three generic words ("cannot, gateway, whatsapp") made two different subjects
    // collide. The same subject written twice does not stop being one subject because the rows
    // point at different files.
    let mut seen_rows: Vec<(String, usize, HashSet<String>)> = Vec::new();
    for target in &targets {
        let Ok(text) = fs::read_to_string(root.join(target)) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let i = i + 1;
            let Some(caps) = row_re.captures(line) else {
                continue;
            };
            let mut fact = line.strip_prefix("- **").unwrap_or(line).to_string();
            if fact.ends_with("**") {
                fact.truncate(fact.len() - 2);
            }
            let fact = fact.split(" — `").next().unwrap_or("").to_string();
            let words = words_of(&fact);
            let key = caps[1].to_string();
            for (okey, oj, owords) in &seen_rows {
                let mut shared: Vec<&String> = words.intersection(owords).collect();
                let need = if *okey == key { 2 } else { 4 };
                if shared.len() >= need {
                    shared.sort();
                    findings.push(format!("{target}:{i}: same subject as line {oj} ({}) — one row per subject; edit the existing row or supersede it (docs/runbooks/memory.md)", shared.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
                }
            }
            seen_rows.push((key, i, words));
        }
    }

    // [5] and orphans: every part is pointed at, and a §slug label under a carrying heading is a duplicate.
    let mut parts: Vec<String> = Vec::new();
    for dir in ["docs/memory-details", "docs/lessons"] {
        if let Ok(rd) = fs::read_dir(root.join(dir)) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.ends_with(".md") {
                    parts.push(format!("{dir}/{n}"));
                }
            }
        }
    }
    parts.sort();
    let lesson_index = fs::read_to_string(root.join("docs/lesson-details.md")).unwrap_or_default();
    for part in &parts {
        if !pointed.contains(part) {
            findings.push(format!("{part}: no index row points at this part (orphan)"));
        }
        let text = match fs::read_to_string(root.join(part)) {
            Ok(t) => t,
            Err(e) => {
                findings.push(format!(
                    "{part}: cannot be read ({e}) — this check cannot see it"
                ));
                continue;
            }
        };
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if slug_re.is_match(line)
                && (i.saturating_sub(3)..i).any(|j| lines[j].starts_with("### "))
            {
                findings.push(format!("{part}:{}: a `§slug` label sits under a heading that already carries the address", i + 1));
            }
        }
        // The lessons index is the map to the entries: an entry with no row there is written but
        // lost to retrieval (the same failure class as the hard-won pointers that went dangling).
        // Rows without entries are the forward direction, covered above when the row is read.
        if part.starts_with("docs/lessons/") {
            for (i, line) in lines.iter().enumerate() {
                if let Some(slug) = line.strip_prefix("### §") {
                    if !lesson_index.contains(&format!("| §{slug} |")) {
                        findings.push(format!(
                            "{part}:{}: §{slug} has no row in docs/lesson-details.md — the entry exists but nothing can find it",
                            i + 1
                        ));
                    }
                }
            }
        }
    }

    // The last rule: a credential-shaped value anywhere in the belief corpus or the diary. The
    // save gate runs this check after every write, so a secret that lands here is refused at once.
    findings.extend(secret_findings(root));

    for f in &findings {
        println!("{f}");
    }
    println!(
        "indexes {} · fact rows {} (cap {}) · detail rows {} · findings {}",
        targets.len(),
        fact_rows,
        MAX,
        detail_rows,
        findings.len()
    );
    Ok(if findings.is_empty() { 0 } else { 1 })
}

/// The old `check-memory-index.sh --selftest`: a temp fixture must still produce the unparseable-row finding.
pub fn selftest() -> Result<i32, Box<dyn Error>> {
    let dir = std::env::temp_dir().join(format!("dsh-optimisemd-selftest-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("docs"))?;
    fs::write(dir.join("docs/hard-won.md"), "# Landmines\n")?;
    fs::write(
        dir.join("MEMORY.md"),
        "# MEMORY.md\n\n## Subject facts\n- **A well formed row.** (2026-10-25) — `docs/hard-won.md`\n- **A row with text after its pointer.** — `docs/hard-won.md` (round 69)\n",
    )?;
    let out = std::process::Command::new(std::env::current_exe()?)
        .arg("--root")
        .arg(&dir)
        .arg("check")
        .output()?;
    let code = out.status.code().unwrap_or(-1);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
    if code == 1 && text.contains("does not end in a backticked") {
        println!("selftest PASS — the unparseable-row finding still fires");
        Ok(0)
    } else {
        eprintln!("selftest FAIL — the unparseable-row finding did not fire (exit {code})");
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("optimisemd-memindex-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("docs/runbooks")).unwrap();
        fs::write(dir.join("docs/hard-won.md"), "# Landmines\n").unwrap();
        fs::write(dir.join("docs/runbooks/memory.md"), "# runbook\n").unwrap();
        dir
    }

    fn out(run: Result<i32, Box<dyn Error>>) -> i32 {
        run.unwrap()
    }

    #[test]
    fn an_unreadable_index_fails_the_gate() {
        let dir = fixture("unreadable");
        fs::write(dir.join("MEMORY.md"), [0xff, 0xfe, 0x00, 0x9f]).unwrap();
        // A check that cannot read its input must fail, not claim zero findings.
        assert_eq!(out(run(&dir, vec![])), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_lesson_entry_without_an_index_row_is_a_finding() {
        let dir = fixture("lesson-lost");
        fs::create_dir_all(dir.join("docs/lessons")).unwrap();
        fs::write(dir.join("MEMORY.md"), "# MEMORY.md\n\n## Subject facts\n").unwrap();
        fs::write(
            dir.join("docs/lessons/01-lessons-1-onward.md"),
            "# Lesson details — entries 1 onward\n\n### §kept\n\n- **Kept (09-23).** Story.\n\n### §lost\n\n- **Lost (09-23).** Story.\n",
        )
        .unwrap();
        let rows = |lost: bool| {
            format!(
                "# Lesson details\n\n| § | File |\n|---|---|\n| §kept | `docs/lessons/01-lessons-1-onward.md` |{}\n",
                if lost {
                    "\n| §lost | `docs/lessons/01-lessons-1-onward.md` |"
                } else {
                    ""
                }
            )
        };
        fs::write(dir.join("docs/lesson-details.md"), rows(false)).unwrap();
        assert_eq!(
            out(run(&dir, vec![])),
            1,
            "an entry nothing points at is a lost lesson and must fail the gate"
        );
        fs::write(dir.join("docs/lesson-details.md"), rows(true)).unwrap();
        assert_eq!(
            out(run(&dir, vec![])),
            0,
            "with both rows the store is consistent"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cap_counts_the_fact_text_not_the_markup_and_date() {
        let dir = fixture("cap");
        let long = "x".repeat(195);
        fs::write(
            dir.join("MEMORY.md"),
            format!("# MEMORY.md\n\n## Subject facts\n- **{long}.** (2026-01-01) — `docs/hard-won.md`\n"),
        )
        .unwrap();
        assert_eq!(
            out(run(&dir, vec![])),
            0,
            "195 chars of text is under the cap"
        );
        let too_long = "y".repeat(205);
        fs::write(
            dir.join("MEMORY.md"),
            format!("# MEMORY.md\n\n## Subject facts\n- **{too_long}.** (2026-01-01) — `docs/hard-won.md`\n"),
        )
        .unwrap();
        assert_eq!(
            out(run(&dir, vec![])),
            1,
            "205 chars of text is over the cap"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_undated_row_is_a_finding() {
        let dir = fixture("undated");
        fs::write(
            dir.join("MEMORY.md"),
            "# MEMORY.md\n\n## Subject facts\n- **A fact without a date.** — `docs/hard-won.md`\n",
        )
        .unwrap();
        assert_eq!(out(run(&dir, vec![])), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_index_caps_at_200_rows() {
        let dir = fixture("rows");
        let mut body = String::from("# MEMORY.md\n\n## Subject facts\n");
        for i in 0..201u32 {
            // One distinctive word per row: only the row count may fire here.
            let word = format!(
                "fact{}{}{}",
                char::from(b'a' + (i / 676) as u8),
                char::from(b'a' + ((i / 26) % 26) as u8),
                char::from(b'a' + (i % 26) as u8),
            );
            body.push_str(&format!(
                "- **{word}.** (2026-01-01) — `docs/hard-won.md`\n"
            ));
        }
        fs::write(dir.join("MEMORY.md"), body).unwrap();
        assert_eq!(out(run(&dir, vec![])), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_subject_across_two_pointers_is_still_one_subject() {
        let dir = fixture("subject");
        fs::write(
            dir.join("MEMORY.md"),
            "# MEMORY.md\n\n## Subject facts\n\
             - **NetDent reception sign-in lockout needs the SQL route restored quickly.** (2026-01-01) — `docs/hard-won.md`\n\
             - **NetDent reception sign-in lockout needs the backup route restored quickly.** (2026-01-02) — `docs/runbooks/memory.md`\n",
        )
        .unwrap();
        assert_eq!(out(run(&dir, vec![])), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn three_generic_shared_words_are_not_the_same_subject() {
        // Calibrated 2026-11-16 on the real index: "cannot, gateway, whatsapp" joined two rows about
        // different things. Four shared distinctive words are the cross-pointer bar.
        let dir = fixture("generic");
        fs::write(
            dir.join("docs/hard-won.md"),
            "# Landmines\n\nwhatsapp gateway disabled linked session\n",
        )
        .unwrap();
        fs::write(
            dir.join("docs/runbooks/memory.md"),
            "# runbook\n\nclaude answers whatsapp gateway plugin\n",
        )
        .unwrap();
        fs::write(
            dir.join("MEMORY.md"),
            "# MEMORY.md\n\n## Subject facts\n\
             - **The VPS copy of the WhatsApp gateway is disabled and cannot steal the linked session.** (2026-01-01) — `docs/hard-won.md`\n\
             - **Claude Code answers WhatsApp through a gateway plugin and cannot host the link or the cron clock.** (2026-01-02) — `docs/runbooks/memory.md`\n",
        )
        .unwrap();
        assert_eq!(out(run(&dir, vec![])), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_row_whose_subject_words_left_their_file_is_a_finding() {
        let dir = fixture("stale");
        fs::write(
            dir.join("MEMORY.md"),
            "# MEMORY.md\n\n## Subject facts\n\
             - **The settlement runbook moved to the invoices route.** (2026-01-01) — `docs/hard-won.md`\n",
        )
        .unwrap();
        // Shaped right, but no word of the row survives in the file it points at.
        assert_eq!(
            out(run(&dir, vec![])),
            1,
            "a row with no distinctive word in its file is stale"
        );
        fs::write(
            dir.join("docs/hard-won.md"),
            "# Landmines\n\nThe settlement invoices runbook survives.\n",
        )
        .unwrap();
        assert_eq!(
            out(run(&dir, vec![])),
            0,
            "the same row passes once its subject is back in the file"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_credential_value_is_a_finding_and_never_printed() {
        let dir = fixture("secret");
        let fake = "sk-FAKE1234567890abcdefghij";
        fs::write(
            dir.join("docs/hard-won.md"),
            format!("# Landmines\n\nThe api_key = \"{fake}\"\n"),
        )
        .unwrap();
        fs::create_dir_all(dir.join("memory")).unwrap();
        fs::write(
            dir.join("memory/2026-01-01.md"),
            format!("## event\npassword: {fake}\n"),
        )
        .unwrap();
        let found = secret_findings(&dir);
        assert!(
            !found.is_empty(),
            "a credential-shaped value in a belief file and in the diary is a finding"
        );
        for f in &found {
            assert!(
                !f.contains(fake),
                "the finding names the file, the line and the key — never the value"
            );
        }
        fs::write(
            dir.join("docs/hard-won.md"),
            "# Landmines\n\napi_key: \"local\"\n\ntoken = xxxx-xxxx-xxxx-redacted\n\nRequestVerificationToken: document.querySelector('input')\n",
        )
        .unwrap();
        fs::remove_file(dir.join("memory/2026-01-01.md")).unwrap();
        assert!(
            secret_findings(&dir).is_empty(),
            "placeholders, short values and code that merely names a token pass"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_declared_address_is_skipped_not_hidden() {
        let dir = fixture("baseline");
        let fake = "sk-FAKE1234567890abcdefghij";
        fs::create_dir_all(dir.join("memory/logs")).unwrap();
        fs::write(
            dir.join("memory/2026-01-01.md"),
            format!("## event\npassword: {fake}\n"),
        )
        .unwrap();
        fs::write(
            dir.join("docs/secret-scan-baseline.txt"),
            "# declared\nmemory/2026-01-01.md:2\n",
        )
        .unwrap();
        assert!(
            secret_findings(&dir).is_empty(),
            "a declared file:line address is skipped"
        );
        fs::write(
            dir.join("docs/secret-scan-baseline.txt"),
            "# nothing declared\n",
        )
        .unwrap();
        assert_eq!(
            secret_findings(&dir).len(),
            1,
            "the same line is a finding again without its declaration"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
