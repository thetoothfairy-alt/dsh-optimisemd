//! Promotion and demotion, ported from the workspace's shell tool with its guards intact.
//!
//! DEMOTE (the only automatic direction): a row whose date is older than the window, whose pointer is a
//! `docs/memory-details/` part, and which is not in a protected section, moves into that part under a dated heading.
//! Nothing is deleted; the index and the part both change and the caller keeps a backup.
//! PROMOTE is reported, never applied: a part section whose words are active in the daily logs but absent from the index.

use crate::repo::RepoError;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

pub const PROTECTED: [&str; 3] = ["Read these first", "Owner preferences", "Landmines"];
pub const DEMOTED_HEADING: &str = "## Demoted from the index (dated)";
pub const DEMOTED_PARAGRAPH: &str = "Facts that stopped being used often enough to earn a line in `MEMORY.md`. The row was moved, not\nremoved: the statement is kept verbatim, with the date it left the index.";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Row {
    pub section: String,
    pub text: String,
    pub date: String,
    pub pointer: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PromoteCandidate {
    pub part: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub rows: usize,
    pub days: u32,
    pub demote: Vec<Row>,
    pub promote: Vec<PromoteCandidate>,
    pub applied: Vec<String>,
    pub dry_run: bool,
    pub backup: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PromoError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("repo: {0}")]
    Repo(#[from] RepoError),
    #[error("row parse: {0}")]
    Parse(String),
}

/// Days since the Unix epoch for a YYYY-MM-DD date (civil calendar, no dependency).
pub fn day_number(date: &str) -> Option<i64> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y: i64 = date[0..4].parse().ok()?;
    let m: i64 = date[5..7].parse().ok()?;
    let d: i64 = date[8..10].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

pub fn today_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs / 86_400;
    // Civil-from-days (Howard Hinnant), the inverse of day_number.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn parse_row(line: &str, section: &str) -> Option<Row> {
    let rest = line.strip_prefix("- **")?;
    let mid = rest.find("** (")?;
    let text = rest[..mid].to_string();
    let after = &rest[mid + 4..];
    let close = after.find(')')?;
    let date = after[..close].to_string();
    day_number(&date)?;
    let tail = &after[close + 1..];
    let pstart = tail.find("`")?;
    let pend = tail[pstart + 1..].find('`')?;
    let pointer = tail[pstart + 1..pstart + 1 + pend].to_string();
    Some(Row {
        section: section.to_string(),
        text,
        date,
        pointer,
    })
}

pub fn parse_rows(body: &str) -> Vec<Row> {
    let mut section = String::new();
    let mut rows = Vec::new();
    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            section = heading.trim().to_string();
            continue;
        }
        if let Some(row) = parse_row(line, &section) {
            rows.push(row);
        }
    }
    rows
}

fn protected(section: &str) -> bool {
    PROTECTED.iter().any(|p| section.starts_with(p))
}

pub struct Promoter {
    root: PathBuf,
}

impl Promoter {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn read(&self, rel: &str) -> Result<String, PromoError> {
        Ok(fs::read_to_string(self.root.join(rel))?)
    }

    pub fn report(&self, days: u32) -> Result<Report, PromoError> {
        let body = self.read("MEMORY.md")?;
        let rows = parse_rows(&body);
        let today = day_number(&today_utc()).unwrap_or(0);
        let demote: Vec<Row> = rows
            .iter()
            .filter(|r| !protected(&r.section))
            .filter(|r| r.pointer.starts_with("docs/memory-details/"))
            .filter(|r| {
                day_number(&r.date)
                    .map(|d| today - d >= days as i64)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        let mut promote = Vec::new();
        if let Ok(entries) = fs::read_dir(self.root.join("docs/memory-details")) {
            for e in entries.flatten() {
                let path = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                for line in text.lines() {
                    let Some(slug) = line.strip_prefix("### §") else {
                        continue;
                    };
                    let slug = slug.trim().to_string();
                    let words: Vec<String> = slug
                        .split(['-', '_', ' '])
                        .filter(|w| w.len() >= 4)
                        .map(|w| w.to_lowercase())
                        .collect();
                    if words.len() < 2 {
                        continue;
                    }
                    let in_index = rows
                        .iter()
                        .any(|r| words.iter().all(|w| r.text.to_lowercase().contains(w)));
                    if in_index {
                        continue;
                    }
                    if self.active_in_logs(&words, days)? {
                        promote.push(PromoteCandidate {
                            part: name.clone(),
                            slug,
                        });
                    }
                }
            }
        }
        Ok(Report {
            rows: rows.len(),
            days,
            demote,
            promote,
            applied: vec![],
            dry_run: true,
            backup: None,
        })
    }

    fn active_in_logs(&self, words: &[String], days: u32) -> Result<bool, PromoError> {
        let today = day_number(&today_utc()).unwrap_or(0);
        let dir = self.root.join("memory");
        let Ok(entries) = fs::read_dir(&dir) else {
            return Ok(false);
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(".md") else {
                continue;
            };
            let Some(d) = day_number(stem) else { continue };
            if today - d > days as i64 {
                continue;
            }
            let text = fs::read_to_string(e.path())
                .unwrap_or_default()
                .to_lowercase();
            if words.iter().all(|w| text.contains(w)) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Move the candidates into their parts. Returns the report, with the backup directory when anything was written.
    pub fn apply(&self, days: u32, dry_run: bool) -> Result<Report, PromoError> {
        let mut report = self.report(days)?;
        report.dry_run = dry_run;
        if dry_run || report.demote.is_empty() {
            return Ok(report);
        }
        let stamp = format!(
            "promotion-apply-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
        let backup = self.root.join("memory/logs").join(&stamp);
        fs::create_dir_all(&backup)?;
        fs::copy(self.root.join("MEMORY.md"), backup.join("MEMORY.md"))?;
        let today = today_utc();
        let mut body = self.read("MEMORY.md")?;
        for row in &report.demote {
            let part_rel = &row.pointer;
            let part_path = self.root.join(part_rel);
            let mut part = if part_path.exists() {
                fs::read_to_string(&part_path)?
            } else {
                continue;
            };
            if let Some(file) = part_path.file_name() {
                fs::copy(&part_path, backup.join(file))?;
            }
            if !part.contains(DEMOTED_HEADING) {
                if !part.ends_with('\n') {
                    part.push('\n');
                }
                part.push_str(&format!("\n{DEMOTED_HEADING}\n\n{DEMOTED_PARAGRAPH}\n"));
            }
            let old_line = format!("- **{}** ({}) — `{}`", row.text, row.date, row.pointer);
            let moved = format!(
                "- **{}** — demoted {today} from `MEMORY.md` (row dated {}); the fact lives here.",
                row.text, row.date
            );
            part.push_str(&moved);
            part.push('\n');
            fs::write(&part_path, part)?;
            // Remove the line with its newline; replacing the text alone leaves a blank line where the row was.
            let with_newline = format!("{old_line}\n");
            if body.contains(&with_newline) {
                body = body.replace(&with_newline, "");
            } else {
                body = body.replace(&old_line, "");
            }
            report.applied.push(format!("{part_rel}: {moved}"));
        }
        // collapse doubled blank lines left behind, then write the index
        let cleaned: Vec<&str> = body.lines().collect();
        let mut out = String::new();
        let mut blank = 0;
        for l in cleaned {
            if l.trim().is_empty() {
                blank += 1;
                if blank > 1 {
                    continue;
                }
            } else {
                blank = 0;
            }
            out.push_str(l);
            out.push('\n');
        }
        let trimmed = out.trim_end().to_string() + "\n";
        fs::write(self.root.join("MEMORY.md"), trimmed)?;
        report.backup = Some(backup.display().to_string());
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("optimisemd-promo-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("docs/memory-details")).unwrap();
        fs::create_dir_all(dir.join("memory")).unwrap();
        fs::write(
            dir.join("docs/memory-details/01.md"),
            "# Memory details — 01\n\n### §test-subject\n\nLong form.\n",
        )
        .unwrap();
        fs::write(
            dir.join("memory/2026-11-13.md"),
            "# 2026-11-13\n\n- today's line\n",
        )
        .unwrap();
        fs::write(dir.join("MEMORY.md"), "# MEMORY.md\n\n## Read these first\n- **A protected row.** (2020-01-01) — `docs/memory-details/01.md`\n\n## Subject facts\n- **A quiet row.** (2020-01-01) — `docs/memory-details/01.md`\n- **A fresh row.** (2026-11-13) — `docs/memory-details/01.md`\n- **A runbook row.** (2020-01-01) — `docs/runbooks/memory.md`\n").unwrap();
        dir
    }

    #[test]
    fn only_the_quiet_detail_row_is_a_demote_candidate() {
        let dir = fixture("report");
        let r = Promoter::new(&dir).report(180).unwrap();
        assert_eq!(r.rows, 4);
        assert_eq!(r.demote.len(), 1);
        assert_eq!(r.demote[0].text, "A quiet row.");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_moves_the_row_into_the_part_and_keeps_a_backup() {
        let dir = fixture("apply");
        let r = Promoter::new(&dir).apply(180, false).unwrap();
        assert_eq!(r.applied.len(), 1);
        let index = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        assert!(!index.contains("A quiet row."));
        assert!(index.contains("A protected row."));
        assert!(index.contains("A fresh row."));
        let part = fs::read_to_string(dir.join("docs/memory-details/01.md")).unwrap();
        assert!(part.contains(DEMOTED_HEADING));
        assert!(part.contains("demoted"));
        assert!(part.contains("A quiet row."));
        let backup = PathBuf::from(r.backup.unwrap());
        assert!(backup.join("MEMORY.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dry_run_writes_nothing() {
        let dir = fixture("dry");
        let before = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        let r = Promoter::new(&dir).apply(180, true).unwrap();
        assert!(r.dry_run);
        assert_eq!(fs::read_to_string(dir.join("MEMORY.md")).unwrap(), before);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn day_numbers_round_trip() {
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(
            day_number("1970-01-02").unwrap() - day_number("1970-01-01").unwrap(),
            1
        );
        assert_eq!(
            day_number("2024-03-01").unwrap() - day_number("2024-02-28").unwrap(),
            2
        ); // leap day
        assert_eq!(
            day_number("2023-03-01").unwrap() - day_number("2023-02-28").unwrap(),
            1
        );
        assert_eq!(day_number("2026-13-01"), None);
        assert!(day_number(&today_utc()).is_some());
    }
}
