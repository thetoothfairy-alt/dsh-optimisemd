//! Lesson creation: the one writing path for `docs/lessons/`. One call appends the entry to the active part, writes
//! its `§slug` row into `docs/lesson-details.md`, derives the slug from the title and rotates the part when the write
//! would cross the part's budget — then runs both gates and rolls every file back on failure. It exists so "the lesson
//! store is full" can never be a reason to lose a lesson: a full part closes and the next one opens by itself.
//!
//! `bin/dsh-optimisemd lesson add --title "…" --text "…" --date MM-DD [--ref "owner|R-146"] [--slug a-slug]`

use std::error::Error;
use std::fs;
use std::path::Path;

pub struct AddArgs<'a> {
    pub title: &'a str,
    pub text: &'a str,
    pub date: &'a str,
    pub ref_tag: Option<&'a str>,
    pub slug: Option<&'a str>,
}

fn one_line(s: &str) -> bool {
    !s.contains('\n') && !s.contains('\r')
}

fn slug_ok(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The slug is the entry's own title, kebab-cased: alphanumeric characters kept lowercased, whitespace runs
/// become one dash, and every other character is dropped — so "(09-13, owner)" ends the slug as `0913-owner`,
/// the same shape the index documents. No length cap: the address is the label.
fn kebab(title: &str) -> String {
    let mut out = String::new();
    let mut pending = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending && !out.is_empty() {
                out.push('-');
            }
            pending = false;
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() {
            pending = !out.is_empty();
        }
    }
    out
}

fn taken(root: &Path, slug: &str) -> bool {
    // Exact addresses only: `### §a-slug` must not claim `### §a-slug-2`.
    if fs::read_to_string(root.join("docs/lesson-details.md"))
        .unwrap_or_default()
        .lines()
        .any(|l| l.starts_with("| §") && l.contains(" | `") && l.contains(&format!("§{slug} |")))
    {
        return true;
    }
    let dir = root.join("docs/lessons");
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = dir.join(e.file_name());
            if p.extension().map(|x| x == "md").unwrap_or(false)
                && fs::read_to_string(&p)
                    .unwrap_or_default()
                    .lines()
                    .any(|l| l == format!("### §{slug}"))
            {
                return true;
            }
        }
    }
    false
}

/// The name of a new lessons part: `NN-lessons-<first>-onward.md`. The range is not baked into the
/// name as an end — a name like `05-lessons-121-onward.md` stays true when lesson 400 lands.
fn next_part_name(root: &Path, first: usize) -> String {
    let mut maxn = 0u32;
    if let Ok(rd) = fs::read_dir(root.join("docs/lessons")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some((pfx, _)) = n.split_once('-') {
                if let Ok(v) = pfx.parse::<u32>() {
                    maxn = maxn.max(v);
                }
            }
        }
    }
    format!("{:02}-lessons-{}-onward.md", maxn + 1, first)
}

fn lesson_count(root: &Path) -> usize {
    let mut n = 0;
    if let Ok(rd) = fs::read_dir(root.join("docs/lessons")) {
        for e in rd.flatten() {
            let p = root.join("docs/lessons").join(e.file_name());
            if p.extension().map(|x| x == "md").unwrap_or(false) {
                n += fs::read_to_string(&p)
                    .unwrap_or_default()
                    .matches("\n### §")
                    .count();
            }
        }
    }
    n
}

pub fn add(root: &Path, a: AddArgs) -> Result<i32, Box<dyn Error>> {
    let title = a.title.trim();
    let text = a.text.trim();
    let date = a.date.trim();
    if title.is_empty() || text.is_empty() || date.is_empty() {
        return Err("title, text and date are all required".into());
    }
    if !one_line(title) || !one_line(text) || !one_line(date) {
        return Err("title, text and date must each be one line".into());
    }
    let ref_tag = a.ref_tag.map(str::trim).filter(|s| !s.is_empty());
    if ref_tag.is_some_and(|r| !one_line(r)) {
        return Err("ref must be one line".into());
    }
    let composed = match ref_tag {
        Some(r) => format!("{title} ({date}, {r})"),
        None => format!("{title} ({date})"),
    };
    // An explicit slug is the writer's call and must be free; a derived one may add a suffix so a
    // save is never refused over a name.
    let slug = match a.slug.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => {
            if !slug_ok(s) {
                return Err(format!(
                    "slug must be lowercase letters, digits and dashes, got {:?}",
                    s
                )
                .into());
            }
            if taken(root, s) {
                return Err(format!("§{s} is already recorded").into());
            }
            s.to_string()
        }
        None => {
            let base = kebab(&composed);
            if base.is_empty() {
                return Err("the title leaves no slug characters — pass --slug".into());
            }
            let mut s = base.clone();
            let mut n = 2;
            while taken(root, &s) {
                s = format!("{base}-{n}");
                n += 1;
            }
            s
        }
    };

    let lessons = root.join("docs/lessons");
    if !lessons.exists() {
        fs::create_dir_all(&lessons)?;
    }
    let index = root.join("docs/lesson-details.md");
    if !index.exists() {
        return Err("docs/lesson-details.md is missing".into());
    }
    let index_before = fs::read_to_string(&index)?;

    let mut parts: Vec<(u32, String)> = Vec::new();
    if let Ok(rd) = fs::read_dir(&lessons) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.ends_with(".md") {
                if let Some((pfx, _)) = n.split_once('-') {
                    if let Ok(v) = pfx.parse::<u32>() {
                        parts.push((v, n));
                    }
                }
            }
        }
    }
    parts.sort();

    let guard_text =
        fs::read_to_string(root.join("scripts/check-context-files.sh")).unwrap_or_default();
    let entry = format!("\n### §{slug}\n\n- **{composed}.** {text}\n");

    // (absolute path, content after, content before — None when the file is created by this call)
    let mut writes: Vec<(std::path::PathBuf, String, Option<String>)> = Vec::new();
    let target_rel: String;

    let part_after: String;
    let part_path: std::path::PathBuf;
    let mut created = false;
    if let Some((_, last)) = parts.last() {
        let before = fs::read_to_string(lessons.join(last))?;
        let budget = crate::slim::budget_for(&guard_text, &format!("docs/lessons/{last}"));
        if crate::slim::crosses_budget(&before, &entry, budget) {
            // The part is full: close it byte-for-byte and open the next. Nothing is moved.
            let name = next_part_name(root, lesson_count(root) + 1);
            let heading = format!(
                "# Lesson details — entries {} onward\n\nPart of the detail behind `lesson-details.md`; the `§slug` pointers there land in this file.\n\n---\n",
                lesson_count(root) + 1
            );
            part_after = heading + &entry;
            part_path = lessons.join(&name);
            target_rel = format!("docs/lessons/{name}");
            created = true;
        } else {
            part_after = before.clone() + &entry;
            part_path = lessons.join(last);
            target_rel = format!("docs/lessons/{last}");
            writes.push((part_path.clone(), part_after.clone(), Some(before)));
        }
    } else {
        let name = next_part_name(root, 1);
        part_after = String::from(
            "# Lesson details — entries 1 onward\n\nPart of the detail behind `lesson-details.md`; the `§slug` pointers there land in this file.\n\n---\n"
        ) + &entry;
        part_path = lessons.join(&name);
        target_rel = format!("docs/lessons/{name}");
        created = true;
    }
    if created {
        writes.push((part_path.clone(), part_after, None));
    }

    // The index row goes after the table's last row, in the same write set as the entry: a lesson
    // recorded in a part nobody can find is a lost lesson.
    let row = format!("| §{slug} | `{target_rel}` |");
    let mut lines: Vec<&str> = index_before.lines().collect();
    let mut at = lines.len();
    for (i, l) in lines.iter().enumerate() {
        // The table's own header row also starts with "| §" — only real rows (a backticked file)
        // set the insert point.
        if l.starts_with("| §") && l.contains(" | `") {
            at = i + 1;
        }
    }
    let mut index_after = String::new();
    for (i, l) in lines.drain(..).enumerate() {
        index_after.push_str(l);
        index_after.push('\n');
        if i + 1 == at {
            index_after.push_str(&row);
            index_after.push('\n');
        }
    }
    if at == 0 {
        // no rows yet: the table header ends at its |---| line; append after the whole file
        index_after.push_str(&row);
        index_after.push('\n');
    }
    writes.push((index.clone(), index_after, Some(index_before)));

    // Write, gate, roll back — one transaction over every touched file.
    for (p, after, _) in &writes {
        optimisemd_core::write_atomic(p, after)?;
    }
    let touched: Vec<String> = writes
        .iter()
        .map(|(p, _, _)| {
            p.strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .to_string()
        })
        .collect();
    let mut ok = true;
    let mut why = String::new();
    for script in [
        "scripts/check-context-files.sh",
        "scripts/check-memory-index.sh",
    ] {
        match optimisemd_core::util::run_script(&root.join(script), root) {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                ok = false;
                why = format!("{script} exited {}", o.status);
                for l in String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| l.contains("FAIL"))
                    .take(5)
                {
                    eprintln!("{l}");
                }
            }
            Err(e) => {
                ok = false;
                why = format!("{script} could not run: {e}");
            }
        }
        if !ok {
            break;
        }
    }
    if !ok {
        for (p, _, before) in &writes {
            match before {
                Some(b) => optimisemd_core::write_atomic(p, b)?,
                None => {
                    let _ = fs::remove_file(p);
                }
            }
        }
        let _ = optimisemd_core::audit::record(root, "lesson", "rolled-back", &touched);
        return Err(format!(
            "the gates failed ({why}); every file was restored — no lesson was recorded"
        )
        .into());
    }
    let _ = optimisemd_core::audit::record(root, "lesson", "applied", &touched);
    if created {
        println!(
            "recorded §{slug} in {target_rel} — the previous part closed at budget and stays byte-for-byte"
        );
    } else {
        println!("recorded §{slug} in {target_rel}");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {

    /// The gates are shell scripts: a host with no POSIX shell cannot run them. Announce the skip —
    /// and fail when REQUIRE_SHELL=1, so a runner that loses its shell is a red run.
    fn no_shell(what: &str) {
        if std::env::var("REQUIRE_SHELL").as_deref() == Ok("1") {
            panic!("{what} needs a POSIX shell and this host has none (REQUIRE_SHELL=1)");
        }
        eprintln!(
            "SKIPPED {what}: no POSIX shell on this host (REQUIRE_SHELL=1 makes that a failure)"
        );
    }

    use super::*;
    use std::path::PathBuf;

    fn fixture(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("optimisemd-lesson-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("docs/lessons")).unwrap();
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::write(
            dir.join("docs/lessons/01-lessons-1-onward.md"),
            "# Lesson details — entries 1 onward\n\nPart of the detail behind `lesson-details.md`.\n\n---\n",
        )
        .unwrap();
        fs::write(
            dir.join("docs/lesson-details.md"),
            "# Lesson details — the lessons index\n\n| § | File |\n|---|---|\n",
        )
        .unwrap();
        for g in ["check-context-files.sh", "check-memory-index.sh"] {
            let guard = dir.join("scripts").join(g);
            fs::write(&guard, "#!/usr/bin/env bash\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&guard, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        dir
    }

    fn args<'a>(title: &'a str, text: &'a str) -> AddArgs<'a> {
        AddArgs {
            title,
            text,
            date: "09-23",
            ref_tag: Some("R-1"),
            slug: None,
        }
    }

    #[test]
    fn the_slug_is_the_title_kebabed_the_way_the_index_documents() {
        assert_eq!(
            kebab("Push and sync keep a TIMING LEDGER (09-13, owner)"),
            "push-and-sync-keep-a-timing-ledger-0913-owner"
        );
        assert!(kebab("!!!").is_empty());
    }

    #[test]
    fn fields_cannot_smuggle_headings_or_rows() {
        let dir = fixture("smuggle");
        assert!(add(
            &dir,
            AddArgs {
                title: "fine",
                text: "bad\n### §another",
                date: "09-23",
                ref_tag: None,
                slug: None,
            }
        )
        .is_err());
        assert!(add(
            &dir,
            AddArgs {
                title: "fine",
                text: "fine text",
                date: "09-23",
                ref_tag: None,
                slug: Some("bad slug"),
            }
        )
        .is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_derived_slug_never_loses_the_save_and_an_explicit_one_must_be_free() {
        let dir = fixture("slug");
        let a = args("A lesson worth keeping", "The story.");
        assert_eq!(add(&dir, a).unwrap(), 0);
        // The same explicit slug is refused; the same title again still lands, under a suffixed address.
        let mut b = args("A lesson worth keeping", "The story again.");
        b.slug = Some("a-lesson-worth-keeping-0923-r1");
        assert!(
            add(&dir, b).is_err(),
            "an explicit slug that is taken is refused"
        );
        let mut c = args("A lesson worth keeping", "The story again.");
        c.ref_tag = None;
        assert_eq!(add(&dir, c).unwrap(), 0);
        let index = fs::read_to_string(dir.join("docs/lesson-details.md")).unwrap();
        assert!(
            index.contains("| §a-lesson-worth-keeping-0923-r1 |"),
            "{index}"
        );
        assert!(
            index.contains("| §a-lesson-worth-keeping-0923 |"),
            "{index}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_part_closes_byte_for_byte_and_the_next_one_opens() {
        if !optimisemd_core::util::shell_available() {
            return no_shell("a lesson landing through the gates");
        }
        let dir = fixture("rotate");
        // A budget home the tool and the gate read together.
        fs::write(
            dir.join("scripts/check-context-files.sh"),
            "#!/usr/bin/env bash\nexit 0\nBUDGETS_BLOCK=$(cat <<'BUDGETS'\n('docs/lessons/*.md', 250, 230),\nBUDGETS\n)\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                dir.join("scripts/check-context-files.sh"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let mut a = args("First lesson", "A story long enough to fill the part.");
        a.ref_tag = None;
        assert_eq!(add(&dir, a).unwrap(), 0);
        let before_close =
            fs::read_to_string(dir.join("docs/lessons/01-lessons-1-onward.md")).unwrap();
        let mut b = args(
            "Second lesson",
            "Another story that crosses the part budget.",
        );
        b.ref_tag = None;
        assert_eq!(
            add(&dir, b).unwrap(),
            0,
            "the second save must land, not be refused"
        );
        assert_eq!(
            fs::read_to_string(dir.join("docs/lessons/01-lessons-1-onward.md")).unwrap(),
            before_close,
            "the closed part stays byte-for-byte"
        );
        let index = fs::read_to_string(dir.join("docs/lesson-details.md")).unwrap();
        assert!(
            index.contains("| §second-lesson-0923 | `docs/lessons/02-lessons-2-onward.md` |"),
            "{index}"
        );
        assert!(
            index.contains("| §first-lesson-0923 | `docs/lessons/01-lessons-1-onward.md` |"),
            "{index}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_gate_leaves_nothing_behind() {
        if !optimisemd_core::util::shell_available() {
            return no_shell("a lesson rolled back by the gate");
        }
        let dir = fixture("rollback");
        fs::write(
            dir.join("scripts/check-memory-index.sh"),
            "#!/usr/bin/env bash\necho 'FAIL [x] something'\nexit 1\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                dir.join("scripts/check-memory-index.sh"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let mut a = args("Doomed lesson", "The story.");
        a.ref_tag = None;
        assert!(add(&dir, a).is_err());
        let part = fs::read_to_string(dir.join("docs/lessons/01-lessons-1-onward.md")).unwrap();
        assert!(!part.contains("doomed"), "{part}");
        let index = fs::read_to_string(dir.join("docs/lesson-details.md")).unwrap();
        assert!(!index.contains("doomed"), "{index}");
        let _ = fs::remove_dir_all(&dir);
    }
}
