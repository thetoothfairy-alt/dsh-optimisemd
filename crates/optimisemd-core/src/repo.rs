//! The repository side of memory: a row in `MEMORY.md`, a section in a detail part, a line in the day's log.
//!
//! This is P2 of the redesign — the plugin writes the *repo* stores, not only tier 0. It refuses what the index rule
//! refuses: a row longer than 200 characters, a pointer that is not a detail part / runbook / skill / the landmine index,
//! and a row whose text is already there (the rule is supersede, never duplicate).

use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("duplicate: MEMORY.md already carries a row starting with {0:?}")]
    Duplicate(String),
    #[error("pointer refused: {0:?} is not a docs/memory-details/ part, a runbook, a skill or docs/hard-won.md")]
    Pointer(String),
}

/// What a save wrote, so the tool can report the exact files.
#[derive(Debug, Clone, Serialize)]
pub struct Written {
    pub row: String,
    pub part: Option<String>,
    pub day: Option<String>,
}

pub const MAX_ROW_TEXT: usize = 200;
pub const DEFAULT_SECTION: &str = "## Subject facts";

/// A real calendar date in YYYY-MM-DD: the shape, a month 1-12, and a day that exists in that month. A date that
/// reaches a write must pass this; a lying `date` command (2026-02-30) must not create a file at a wrong path.
pub fn valid_date(date: &str) -> bool {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    if !(b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    let year: i32 = date[..4].parse().unwrap_or(0);
    let month: u32 = date[5..7].parse().unwrap_or(0);
    let day: u32 = date[8..].parse().unwrap_or(0);
    if !(1..=12).contains(&month) || day == 0 {
        return false;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let last = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => 0,
    };
    day <= last
}

pub struct Repo {
    root: PathBuf,
}

fn pointer_ok(p: &str) -> bool {
    // The pointer is written into a row and later joined to the root: it must stay inside the
    // repository, carry no shell of its own, and belong to one of the four documented families.
    if p.contains("..")
        || p.contains('`')
        || p.contains('\n')
        || p.contains('\r')
        || p.contains('\\')
    {
        return false;
    }
    p.starts_with("docs/memory-details/")
        || p.starts_with("docs/runbooks/")
        || p.starts_with("skills/")
        || p == "docs/hard-won.md"
        || p.starts_with("docs/hard-won/")
}

/// A detail-part file name: one path component under `docs/memory-details/`, nothing else. This is
/// joined to the root before writing, so `..`, separators and absolute paths are refused here.
fn part_ok(p: &str) -> bool {
    !p.is_empty()
        && p.ends_with(".md")
        && !p.contains("..")
        && !p.contains('/')
        && !p.contains('\\')
        && !p.contains('\n')
        && !p.contains('\r')
}

fn one_line(s: &str) -> bool {
    !s.contains('\n') && !s.contains('\r')
}

impl Repo {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn memory_path(&self) -> PathBuf {
        self.root.join("MEMORY.md")
    }

    /// The one creation call: a dated index row, optionally its detail section and the day's line.
    #[allow(clippy::too_many_arguments)]
    pub fn save(
        &self,
        text: &str,
        date: &str,
        pointer: &str,
        section: Option<&str>,
        part: Option<&str>,
        slug: Option<&str>,
        detail: Option<&str>,
    ) -> Result<Written, RepoError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(RepoError::Invalid("text is empty".into()));
        }
        if text.chars().count() > MAX_ROW_TEXT {
            return Err(RepoError::Invalid(format!(
                "row text is {} characters; the cap is {MAX_ROW_TEXT} — move the detail into the part",
                text.chars().count()
            )));
        }
        if !one_line(text) {
            return Err(RepoError::Invalid(
                "the row text must be one line — no newline".into(),
            ));
        }
        // Every date that reaches a write is a real calendar date: a wrong one would create a file
        // at a wrong path and date a row untruthfully.
        if !valid_date(date) {
            return Err(RepoError::Invalid(format!(
                "date must be a real YYYY-MM-DD calendar date, got {date:?}"
            )));
        }
        if !pointer_ok(pointer) {
            return Err(RepoError::Pointer(pointer.to_string()));
        }
        if let Some(p) = part {
            if !part_ok(p) {
                return Err(RepoError::Invalid(format!(
                    "part must be one file name like 01-subject.md inside docs/memory-details, got {p:?}"
                )));
            }
        }
        if let Some(s) = slug {
            if s.is_empty()
                || !s
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                || s.contains('\n')
            {
                return Err(RepoError::Invalid(format!(
                    "slug must be lowercase letters, digits and dashes, got {s:?}"
                )));
            }
        }
        if let Some(d) = detail {
            if d.contains("\n##") {
                return Err(RepoError::Invalid(
                    "detail must not smuggle headings (no line starting with ##)".into(),
                ));
            }
        }
        let memory = self.memory_path();
        let body = fs::read_to_string(&memory)?;
        // Duplicate = the same fact already in a ROW. Prose or a heading that happens to quote the
        // words is not an index row, and rows sharing only a long opening are two rows, not one.
        let needle: String = text.chars().take(60).collect();
        for line in body.lines() {
            let Some(rest) = line.strip_prefix("- **") else {
                continue;
            };
            let Some(mid) = rest.find("** (") else {
                continue;
            };
            let t = &rest[..mid];
            if t == text || t.starts_with(&needle) {
                return Err(RepoError::Duplicate(needle));
            }
        }
        let row = format!("- **{text}** ({date}) — `{pointer}`");
        let heading = section.unwrap_or(DEFAULT_SECTION).trim().to_string();
        let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
        let headings: Vec<String> = lines
            .iter()
            .filter(|l| l.starts_with("## "))
            .map(|l| l.trim().to_string())
            .collect();
        match lines.iter().position(|l| l.trim() == heading) {
            Some(i) => {
                let mut j = i + 1;
                while j < lines.len() && !lines[j].starts_with("## ") {
                    j += 1;
                }
                // The row joins the section's content: before any trailing blanks, keeping one
                // blank line between it and the next heading.
                let mut k = j;
                while k > i + 1 && lines[k - 1].trim().is_empty() {
                    k -= 1;
                }
                lines.insert(k, row.clone());
                if k + 1 < lines.len() && lines[k + 1].starts_with("## ") {
                    lines.insert(k + 1, String::new());
                }
            }
            None => {
                // A named section that does not exist yet is created before Subject facts, as before. With no Subject
                // facts either, nothing can place the row safely — name the headings instead of writing a row outside
                // every section (the index gate refuses that, 2026-11-14). The row goes INSIDE its
                // own section, never above its heading.
                let Some(at) = lines.iter().position(|l| l.trim() == DEFAULT_SECTION) else {
                    return Err(RepoError::Invalid(format!(
                        "no `{heading}` section in MEMORY.md — pass section with one of: {}",
                        headings.join(", ")
                    )));
                };
                lines.insert(at, heading.clone());
                lines.insert(at + 1, String::new());
                lines.insert(at + 2, row.clone());
                lines.insert(at + 3, String::new());
            }
        }
        let mut body = lines.join("\n");
        if !body.ends_with('\n') {
            body.push('\n');
        }
        crate::util::write_atomic(&memory, &body)?;

        let mut written = Written {
            row: row.clone(),
            part: None,
            day: None,
        };
        if let (Some(part), Some(slug), Some(detail)) = (part, slug, detail) {
            let path = self.root.join("docs/memory-details").join(part);
            let mut part_body = if path.exists() {
                fs::read_to_string(&path)?
            } else {
                format!("# Memory details — {part}\n\n")
            };
            if !part_body.ends_with('\n') {
                part_body.push('\n');
            }
            part_body.push_str(&format!("\n### §{slug}\n\n{}\n", detail.trim()));
            crate::util::write_atomic(&path, &part_body)?;
            written.part = Some(path.display().to_string());
        }
        self.append_day(date, &format!("{text} — `{pointer}`"))?;
        written.day = Some(self.day_path(date).display().to_string());
        Ok(written)
    }

    /// The creation call with the workspace's checks as the gate (P4): back up what will change, write, run
    /// `scripts/check-memory-index.sh`, and restore the backups if it fails. A save that would land an invalid index is
    /// refused, not reported.
    #[allow(clippy::too_many_arguments)]
    pub fn save_checked(
        &self,
        text: &str,
        date: &str,
        pointer: &str,
        section: Option<&str>,
        part: Option<&str>,
        slug: Option<&str>,
        detail: Option<&str>,
    ) -> Result<Written, RepoError> {
        let stamp = std::process::id();
        let backup = self.root.join("memory/logs").join(format!("save-{stamp}"));
        std::fs::create_dir_all(&backup)?;
        let mem = self.memory_path();
        let part_path = part.map(|p| self.root.join("docs/memory-details").join(p));
        let day_path = self.day_path(date);
        let mut saved: Vec<(PathBuf, PathBuf)> = Vec::new();
        if mem.exists() {
            let dst = backup.join("MEMORY.md");
            std::fs::copy(&mem, &dst)?;
            saved.push((mem.clone(), dst));
        }
        if let Some(p) = &part_path {
            if p.exists() && p.is_file() {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "part.md".into());
                let dst = backup.join(name);
                std::fs::copy(p, &dst)?;
                saved.push((p.clone(), dst));
            }
        }
        if day_path.exists() && day_path.is_file() {
            let name = day_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "day.md".into());
            let dst = backup.join(name);
            std::fs::copy(&day_path, &dst)?;
            saved.push((day_path.clone(), dst));
        }
        // Restore every tracked file and remove the ones this save created. Used both when the write
        // itself fails partway and when the gate refuses the result.
        let restore = |saved: &[(PathBuf, PathBuf)]| -> std::io::Result<()> {
            for (orig, copy) in saved {
                std::fs::copy(copy, orig)?;
            }
            if !saved.iter().any(|(o, _)| o == &day_path) && day_path.exists() {
                std::fs::remove_file(&day_path)?;
            }
            if let Some(p) = &part_path {
                if !saved.iter().any(|(o, _)| o == p) && p.exists() {
                    std::fs::remove_file(p)?;
                }
            }
            Ok(())
        };
        let written = match self.save(text, date, pointer, section, part, slug, detail) {
            Ok(w) => w,
            Err(e) => {
                // A save that fails after MEMORY.md was written must not leave the row behind.
                let _ = restore(&saved);
                let _ =
                    crate::audit::record(&self.root, "save", "rolled-back", &["MEMORY.md".into()]);
                return Err(e);
            }
        };
        let mut touched: Vec<String> = vec!["MEMORY.md".into()];
        if let Some(p) = part {
            touched.push(format!("docs/memory-details/{p}"));
        }
        touched.push(format!("memory/{date}.md"));
        let script = self.root.join("scripts/check-memory-index.sh");
        if !script.exists() {
            let _ = crate::audit::record(&self.root, "save", "applied", &touched);
            return Ok(written);
        }
        // Run the checker directly: the wrapper may be a shim or the binary itself; `bash <binary>` cannot execute.
        let gate = std::process::Command::new(&script)
            .current_dir(&self.root)
            .output();
        if gate.as_ref().map(|o| o.status.success()).unwrap_or(false) {
            let _ = crate::audit::record(&self.root, "save", "applied", &touched);
            return Ok(written);
        }
        if let Err(e) = restore(&saved) {
            let _ = crate::audit::record(&self.root, "save", "rollback-failed", &touched);
            return Err(RepoError::Invalid(format!(
                "the index check failed and the rollback also failed ({e}) — inspect MEMORY.md and the backup at {}",
                backup.display()
            )));
        }
        // Report WHY, not just that it failed: the checker's findings are the reason, and a generic message sends the
        // session hunting through files it may not have touched (the summary line is dropped).
        let findings = gate
            .ok()
            .map(|o| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                )
            })
            .map(|text| {
                text.lines()
                    .filter(|l| !l.trim().is_empty() && !l.starts_with("indexes "))
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        let msg = if findings.is_empty() {
            "the index check failed; the save was rolled back".to_string()
        } else {
            format!("the index check failed; the save was rolled back — {findings}")
        };
        let _ = crate::audit::record(&self.root, "save", "rolled-back", &touched);
        Err(RepoError::Invalid(msg))
    }

    /// Supersede an existing row in place: the new text and date replace the old one, never a second row.
    pub fn supersede(
        &self,
        needle: &str,
        text: &str,
        date: &str,
        pointer: &str,
    ) -> Result<String, RepoError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(RepoError::Invalid("text is empty".into()));
        }
        if text.chars().count() > MAX_ROW_TEXT {
            return Err(RepoError::Invalid(format!(
                "row text is {} characters; the cap is {MAX_ROW_TEXT}",
                text.chars().count()
            )));
        }
        if !one_line(text) {
            return Err(RepoError::Invalid(
                "the row text must be one line — no newline".into(),
            ));
        }
        if !valid_date(date) {
            return Err(RepoError::Invalid(format!(
                "date must be a real YYYY-MM-DD calendar date, got {date:?}"
            )));
        }
        if !pointer_ok(pointer) {
            return Err(RepoError::Pointer(pointer.to_string()));
        }
        let memory = self.memory_path();
        let body = fs::read_to_string(&memory)?;
        // Only rows are candidates: a heading or a paragraph that quotes the words is not a row.
        let hits: Vec<&str> = body
            .lines()
            .filter(|l| l.starts_with("- **") && l.contains(needle))
            .collect();
        if hits.is_empty() {
            return Err(RepoError::Invalid(format!("no row contains {needle:?}")));
        }
        if hits.len() > 1 {
            return Err(RepoError::Invalid(format!(
                "{} rows contain {needle:?}; narrow it",
                hits.len()
            )));
        }
        let old = hits[0];
        let row = format!("- **{text}** ({date}) — `{pointer}`");
        // Line-based replacement: the words may also appear in prose that must not move.
        let new_body: String = body
            .lines()
            .map(|l| if l == old { row.as_str() } else { l })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        crate::util::write_atomic(&memory, &new_body)?;
        let _ = crate::audit::record(
            &self.root,
            "supersede",
            "applied",
            &["MEMORY.md".to_string()],
        );
        Ok(row)
    }

    fn day_path(&self, date: &str) -> PathBuf {
        self.root.join("memory").join(format!("{date}.md"))
    }

    /// Append the day's line; create the day file with its title. Append-only, never rewritten.
    pub fn append_day(&self, date: &str, text: &str) -> Result<PathBuf, RepoError> {
        if !valid_date(date) {
            return Err(RepoError::Invalid(format!(
                "date must be a real YYYY-MM-DD calendar date, got {date:?}"
            )));
        }
        let path = self.day_path(date);
        fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let header = if fs::metadata(&path).map(|m| m.len()).unwrap_or(0) == 0 {
            format!("# {date}\n\n")
        } else {
            String::new()
        };
        file.write_all(header.as_bytes())?;
        file.write_all(format!("- {}\n", text.trim()).as_bytes())?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("optimisemd-repo-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("docs/memory-details")).unwrap();
        fs::create_dir_all(dir.join("memory")).unwrap();
        fs::write(dir.join("MEMORY.md"), "# MEMORY.md\n\n## Read these first\n- **Never touch the live service.** (2026-01-01) — `docs/hard-won.md`\n\n## Subject facts\n- **An existing row.** (2026-01-01) — `docs/memory-details/01.md`\n").unwrap();
        dir
    }

    #[test]
    fn a_missing_section_is_named_not_guessed() {
        let ws = workspace("nosection");
        fs::write(
            ws.join("MEMORY.md"),
            "# MEMORY.md\n\n## Machines\n- **A row.** (2026-01-01) — `docs/runbooks/memory.md`\n",
        )
        .unwrap();
        let repo = Repo::new(&ws);
        let err = repo
            .save(
                "A new fact.",
                "2026-11-14",
                "docs/runbooks/memory.md",
                None,
                None,
                None,
                None,
            )
            .unwrap_err();
        assert!(err.to_string().contains("pass section"), "{err}");
        let after = fs::read_to_string(ws.join("MEMORY.md")).unwrap();
        assert!(
            !after.contains("A new fact."),
            "the row must not be written"
        );
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn a_save_writes_the_row_under_the_section_and_keeps_the_file_valid() {
        let dir = workspace("save");
        let repo = Repo::new(&dir);
        let w = repo
            .save(
                "A new fact that fits the cap.",
                "2026-11-13",
                "docs/memory-details/01.md",
                None,
                Some("01.md"),
                Some("new-fact"),
                Some("The long form of the fact."),
            )
            .unwrap();
        let body = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        assert!(body.contains(
            "- **A new fact that fits the cap.** (2026-11-13) — `docs/memory-details/01.md`"
        ));
        assert!(body.contains("## Subject facts"));
        assert!(w.part.unwrap().ends_with("docs/memory-details/01.md"));
        assert!(w.day.unwrap().ends_with("memory/2026-11-13.md"));
        let part = fs::read_to_string(dir.join("docs/memory-details/01.md")).unwrap();
        assert!(part.contains("### §new-fact"));
        assert!(part.contains("The long form of the fact."));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_duplicate_and_an_over_long_row_and_a_bad_pointer_are_refused() {
        let dir = workspace("dup");
        let repo = Repo::new(&dir);
        assert!(matches!(
            repo.save(
                "An existing row.",
                "2026-11-13",
                "docs/memory-details/01.md",
                None,
                None,
                None,
                None
            ),
            Err(RepoError::Duplicate(_))
        ));
        let long = "x".repeat(MAX_ROW_TEXT + 1);
        assert!(matches!(
            repo.save(
                &long,
                "2026-11-13",
                "docs/memory-details/01.md",
                None,
                None,
                None,
                None
            ),
            Err(RepoError::Invalid(_))
        ));
        assert!(matches!(
            repo.save(
                "A fact.",
                "2026-11-13",
                "notes/elsewhere.md",
                None,
                None,
                None,
                None
            ),
            Err(RepoError::Pointer(_))
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_check_rolls_the_save_back() {
        let dir = workspace("checked");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        let gate = dir.join("scripts/check-memory-index.sh");
        std::fs::write(
            &gate,
            "#!/usr/bin/env bash\necho 'MEMORY.md:3: a planted finding'; echo 'indexes 1 · fact rows 0 · findings 1'; exit 1\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&gate, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let repo = Repo::new(&dir);
        let before = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        let out = repo.save_checked(
            "A fact that must not land.",
            "2026-11-13",
            "docs/memory-details/02.md",
            None,
            Some("02.md"),
            Some("slug"),
            Some("detail"),
        );
        match out {
            Err(RepoError::Invalid(msg)) => {
                assert!(
                    msg.contains("planted finding") && msg.contains("rolled back"),
                    "{msg}"
                )
            }
            other => panic!("expected an Invalid error, got {other:?}"),
        }
        assert_eq!(fs::read_to_string(dir.join("MEMORY.md")).unwrap(), before);
        assert!(!dir.join("docs/memory-details/02.md").exists());
        assert!(!dir.join("memory/2026-11-13.md").exists());
        let log = fs::read_to_string(dir.join("memory/logs/repo-writes.log")).unwrap();
        assert!(log.contains("save rolled-back"), "{log}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_save_gate_runs_a_shim_directly() {
        let dir = workspace("gate-shim");
        let scripts = dir.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        let shim = scripts.join("check-memory-index.sh");
        std::fs::write(
            &shim,
            "#!/usr/bin/env bash\nroot=\"$(cd \"$(dirname \"$0\")/..\" && pwd)\"\nexec \"$root/bin/dsh-optimisemd\" --root \"$root\" check \"$@\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let bin = dir.join("bin/dsh-optimisemd");
        std::fs::write(
            &bin,
            "#!/usr/bin/env bash\necho 'indexes 1 · fact rows 1 · detail rows 0 · findings 0'\nexit 0\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let repo = Repo::new(&dir);
        repo.save_checked(
            "A fact behind a shim.",
            "2026-11-13",
            "docs/memory-details/01.md",
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(std::fs::read_to_string(dir.join("MEMORY.md"))
            .unwrap()
            .contains("A fact behind a shim."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_successful_save_is_audited() {
        let dir = workspace("audit");
        let repo = Repo::new(&dir);
        repo.save_checked(
            "An audited fact.",
            "2026-11-13",
            "docs/memory-details/01.md",
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let log = fs::read_to_string(dir.join("memory/logs/repo-writes.log")).unwrap();
        assert!(log.contains("save applied"), "{log}");
        assert!(log.contains("MEMORY.md"), "{log}");
        assert!(log.contains("memory/2026-11-13.md"), "{log}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn supersede_replaces_exactly_one_row_in_place() {
        let dir = workspace("day");
        let repo = Repo::new(&dir);
        let row = repo
            .supersede(
                "An existing row",
                "An existing row, corrected.",
                "2026-11-13",
                "docs/memory-details/01.md",
            )
            .unwrap();
        let body = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        assert!(body.contains(&row));
        assert_eq!(body.matches("An existing row").count(), 1);
        assert!(repo
            .supersede(
                "existing row, correct",
                "x",
                "2026-11-13",
                "docs/memory-details/01.md"
            )
            .is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_section_lands_before_the_subject_facts() {
        let dir = workspace("supersede");
        let repo = Repo::new(&dir);
        repo.save(
            "A preference the owner stated.",
            "2026-11-13",
            "docs/memory-details/01.md",
            Some("## Owner preferences"),
            None,
            None,
            None,
        )
        .unwrap();
        let body = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        let prefs = body.find("## Owner preferences").unwrap();
        let facts = body.find("## Subject facts").unwrap();
        assert!(prefs < facts, "{body}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_day_line_appends_and_never_rewrites() {
        let dir = workspace("section");
        let repo = Repo::new(&dir);
        let p = repo.append_day("2026-11-13", "first line").unwrap();
        repo.append_day("2026-11-13", "second line").unwrap();
        let body = fs::read_to_string(p).unwrap();
        assert!(body.starts_with("# 2026-11-13"));
        assert!(body.contains("- first line"));
        assert!(body.contains("- second line"));
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod hardening_tests {
    use super::*;

    fn workspace(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("optimisemd-repo-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("docs/memory-details")).unwrap();
        fs::create_dir_all(dir.join("memory")).unwrap();
        fs::write(dir.join("MEMORY.md"), "# MEMORY.md\n\n## Read these first\n- **Never touch the live service.** (2026-01-01) — `docs/hard-won.md`\n\n## Subject facts\n- **An existing row.** (2026-01-01) — `docs/memory-details/01.md`\n").unwrap();
        dir
    }

    #[test]
    fn a_supplied_date_is_calendar_checked() {
        let dir = workspace("baddate");
        let repo = Repo::new(&dir);
        assert!(matches!(
            repo.save(
                "A fact.",
                "2026-02-30",
                "docs/memory-details/01.md",
                None,
                None,
                None,
                None
            ),
            Err(RepoError::Invalid(_))
        ));
        assert!(matches!(
            repo.append_day("2026-99-99", "a line"),
            Err(RepoError::Invalid(_))
        ));
        assert!(matches!(
            repo.supersede(
                "An existing row",
                "Corrected.",
                "2026-13-01",
                "docs/memory-details/01.md"
            ),
            Err(RepoError::Invalid(_))
        ));
        assert!(!dir.join("memory/2026-02-30.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_part_cannot_escape_the_parts_directory() {
        let dir = workspace("escape");
        let repo = Repo::new(&dir);
        assert!(matches!(
            repo.save(
                "A fact.",
                "2026-11-13",
                "docs/memory-details/01.md",
                None,
                Some("../../evil.md"),
                Some("slug"),
                Some("detail")
            ),
            Err(RepoError::Invalid(_))
        ));
        assert!(!dir.join("evil.md").exists());
        // A part that ends in ".." once panicked on file_name().unwrap(); it must refuse instead.
        assert!(repo
            .save_checked(
                "A fact.",
                "2026-11-13",
                "docs/memory-details/01.md",
                None,
                Some(".."),
                Some("slug"),
                Some("detail")
            )
            .is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pointer_cannot_point_outside_the_repo() {
        let dir = workspace("pointer");
        let repo = Repo::new(&dir);
        for pointer in [
            "docs/memory-details/../../x.md",
            "docs/runbooks/../secrets.md",
            "docs/hard-won.md` and more",
            "docs/runbooks/a\nb.md",
        ] {
            assert!(
                matches!(
                    repo.save("A fact.", "2026-11-13", pointer, None, None, None, None),
                    Err(RepoError::Pointer(_)),
                ),
                "pointer {pointer:?} must be refused"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_row_must_be_one_line() {
        let dir = workspace("rowline");
        let repo = Repo::new(&dir);
        assert!(matches!(
            repo.save(
                "A fact\n- **A smuggled row.** (2026-01-01) — `docs/hard-won.md`",
                "2026-11-13",
                "docs/memory-details/01.md",
                None,
                None,
                None,
                None
            ),
            Err(RepoError::Invalid(_))
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_section_holds_its_row() {
        let dir = workspace("newsec");
        let repo = Repo::new(&dir);
        repo.save(
            "A preference the owner stated.",
            "2026-11-13",
            "docs/memory-details/01.md",
            Some("## Owner preferences"),
            None,
            None,
            None,
        )
        .unwrap();
        let body = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        let heading = body.find("## Owner preferences").unwrap();
        let row = body.find("A preference the owner stated.").unwrap();
        let facts = body.find("## Subject facts").unwrap();
        assert!(heading < row && row < facts, "{body}");
        // The row keeps a blank line between itself and the next heading.
        let after_row = &body[row..];
        let next_heading = after_row.find("\n## ").unwrap();
        assert!(after_row[..next_heading].ends_with('\n'), "{body}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_detection_scans_rows_not_prose() {
        let dir = workspace("dupscan");
        let repo = Repo::new(&dir);
        // The new row's opening words appear in prose (the preamble), not in any row: it is not a duplicate.
        let text = "The team library OneDrive stays unlinked after the Dell7050 re-link and is retried monthly";
        let needle: String = text.chars().take(60).collect();
        let body = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        let body = body.replace(
            "# MEMORY.md\n",
            &format!("# MEMORY.md\n\nContext: {needle} …as prose.\n"),
        );
        fs::write(dir.join("MEMORY.md"), body).unwrap();
        repo.save(
            text,
            "2026-11-13",
            "docs/memory-details/01.md",
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn supersede_replaces_only_the_row_line() {
        let dir = workspace("supline");
        let repo = Repo::new(&dir);
        let body = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        let body = body.replace(
            "# MEMORY.md\n",
            "# MEMORY.md\n\nA heading that also mentions An existing row in passing.\n",
        );
        fs::write(dir.join("MEMORY.md"), body).unwrap();
        repo.supersede(
            "An existing row",
            "An existing row, corrected.",
            "2026-11-13",
            "docs/memory-details/01.md",
        )
        .unwrap();
        let after = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        assert!(
            after.contains("A heading that also mentions An existing row in passing."),
            "{after}"
        );
        assert_eq!(after.matches("An existing row, corrected.").count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_checked_rolls_back_when_the_write_itself_fails() {
        let dir = workspace("midfail");
        // The day path is a directory: MEMORY.md is written first, then the day append fails.
        fs::create_dir_all(dir.join("memory/2026-11-13.md")).unwrap();
        let before = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        let out = Repo::new(&dir).save_checked(
            "A fact that must not survive.",
            "2026-11-13",
            "docs/memory-details/01.md",
            None,
            None,
            None,
            None,
        );
        assert!(out.is_err(), "{out:?}");
        assert_eq!(fs::read_to_string(dir.join("MEMORY.md")).unwrap(), before);
        let _ = fs::remove_dir_all(&dir);
    }
}
