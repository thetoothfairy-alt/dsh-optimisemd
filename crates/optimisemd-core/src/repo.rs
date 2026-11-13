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

pub struct Repo {
    root: PathBuf,
}

fn pointer_ok(p: &str) -> bool {
    p.starts_with("docs/memory-details/")
        || p.starts_with("docs/runbooks/")
        || p.starts_with("skills/")
        || p == "docs/hard-won.md"
        || p.starts_with("docs/hard-won/")
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
        if !pointer_ok(pointer) {
            return Err(RepoError::Pointer(pointer.to_string()));
        }
        let memory = self.memory_path();
        let mut body = fs::read_to_string(&memory)?;
        let needle: String = text.chars().take(60).collect();
        if body.contains(&needle) {
            return Err(RepoError::Duplicate(needle));
        }
        let row = format!("- **{text}** ({date}) — `{pointer}`");
        let heading = section.unwrap_or(DEFAULT_SECTION);
        let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
        let at = lines.iter().position(|l| l.trim() == heading.trim());
        match at {
            Some(i) => {
                let mut j = i + 1;
                while j < lines.len() && !lines[j].starts_with("## ") {
                    j += 1;
                }
                lines.insert(j, row.clone());
            }
            None => {
                let at = lines
                    .iter()
                    .position(|l| l.trim() == "## Subject facts")
                    .unwrap_or(lines.len());
                lines.insert(at, row.clone());
                lines.insert(at + 1, String::new());
                lines.insert(at + 2, heading.to_string());
            }
        }
        body = lines.join("\n");
        if !body.ends_with('\n') {
            body.push('\n');
        }
        fs::write(&memory, body)?;

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
            fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
            fs::write(&path, part_body)?;
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
            if p.exists() {
                let dst = backup.join(p.file_name().unwrap());
                std::fs::copy(p, &dst)?;
                saved.push((p.clone(), dst));
            }
        }
        if day_path.exists() {
            let dst = backup.join(day_path.file_name().unwrap());
            std::fs::copy(&day_path, &dst)?;
            saved.push((day_path.clone(), dst));
        }
        let written = self.save(text, date, pointer, section, part, slug, detail)?;
        let script = self.root.join("scripts/check-memory-index.sh");
        if !script.exists() {
            return Ok(written);
        }
        let ok = std::process::Command::new("bash")
            .arg(&script)
            .current_dir(&self.root)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Ok(written);
        }
        // restore every tracked file, and remove the files this save created that had no prior copy
        for (orig, copy) in &saved {
            let _ = std::fs::copy(copy, orig);
        }
        if !saved.iter().any(|(o, _)| o == &day_path) && day_path.exists() {
            let _ = std::fs::remove_file(&day_path);
        }
        if let Some(p) = &part_path {
            if !saved.iter().any(|(o, _)| o == p) && p.exists() {
                let _ = std::fs::remove_file(p);
            }
        }
        Err(RepoError::Invalid(
            "the index check failed; the save was rolled back".into(),
        ))
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
        if !pointer_ok(pointer) {
            return Err(RepoError::Pointer(pointer.to_string()));
        }
        let memory = self.memory_path();
        let body = fs::read_to_string(&memory)?;
        let hits: Vec<&str> = body.lines().filter(|l| l.contains(needle)).collect();
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
        let new_body = body.replace(old, &row);
        fs::write(&memory, new_body)?;
        Ok(row)
    }

    fn day_path(&self, date: &str) -> PathBuf {
        self.root.join("memory").join(format!("{date}.md"))
    }

    /// Append the day's line; create the day file with its title. Append-only, never rewritten.
    pub fn append_day(&self, date: &str, text: &str) -> Result<PathBuf, RepoError> {
        if !date.chars().all(|c| c.is_ascii_digit() || c == '-') || date.len() != 10 {
            return Err(RepoError::Invalid(format!(
                "date must be YYYY-MM-DD, got {date:?}"
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
        std::fs::write(
            dir.join("scripts/check-memory-index.sh"),
            "#!/usr/bin/env bash\nexit 1\n",
        )
        .unwrap();
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
        assert!(matches!(out, Err(RepoError::Invalid(_))));
        assert_eq!(fs::read_to_string(dir.join("MEMORY.md")).unwrap(), before);
        assert!(!dir.join("docs/memory-details/02.md").exists());
        assert!(!dir.join("memory/2026-11-13.md").exists());
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
