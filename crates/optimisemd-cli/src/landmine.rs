//! Landmine creation: the one writing path that used to be a hand-run skill. It appends the trap line to its part, writes the
//! reproduction under `### §<slug>` in `docs/hard-won-details.md`, and runs the context guard as its gate — rolling both files
//! back when the guard fails, the same shape as `memory_save`'s checks gate.

use std::error::Error;
use std::fs;
use std::path::Path;

pub struct AddArgs<'a> {
    pub part: &'a str,
    pub text: &'a str,
    pub context: &'a str,
    pub slug: &'a str,
    pub detail: &'a str,
}

const MAX_LINE: usize = 250;

/// A landmine part file name: one component under `docs/hard-won/`. It is joined to the root before
/// writing, so `..`, separators and absolute paths are refused here (they used to reach the join).
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

pub fn add(root: &Path, a: AddArgs) -> Result<i32, Box<dyn Error>> {
    let part_name = a.part.trim();
    if !part_ok(part_name) {
        return Err(format!(
            "part must be one file name like 01-claims.md inside docs/hard-won, got {:?}",
            a.part
        )
        .into());
    }
    let part = root.join("docs/hard-won").join(part_name);
    let details = root.join("docs/hard-won-details.md");
    if !part.exists() {
        return Err(format!("no such part: {}", part.display()).into());
    }
    if !details.exists() {
        return Err("docs/hard-won-details.md is missing".into());
    }
    let slug = a.slug.trim();
    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!(
            "slug must be lowercase letters, digits and dashes, got {:?}",
            a.slug
        )
        .into());
    }
    // Each field is one line: a newline would smuggle rows or headings past the cap.
    if !one_line(a.text) || !one_line(a.context) || !one_line(a.detail) {
        return Err("text, context and detail must not contain newlines".into());
    }
    let line = format!("- **{}** — {} — §{}", a.text.trim(), a.context.trim(), slug);
    if line.chars().count() > MAX_LINE {
        return Err(format!(
            "line is {} characters; the cap is {MAX_LINE}",
            line.chars().count()
        )
        .into());
    }
    let part_before = fs::read_to_string(&part)?;
    let details_before = fs::read_to_string(&details)?;
    // The slug is unique across BOTH files: a second `### §slug` heading in the details is a
    // duplicate address whichever part the line lives in.
    if part_before.contains(&format!("§{slug}")) || details_before.contains(&format!("### §{slug}"))
    {
        return Err(format!("§{slug} is already recorded").into());
    }
    let mut part_after = part_before.clone();
    if !part_after.ends_with('\n') {
        part_after.push('\n');
    }
    part_after.push_str(&line);
    part_after.push('\n');
    optimisemd_core::write_atomic(&part, &part_after)?;

    let mut details_after = details_before.clone();
    if !details_after.ends_with('\n') {
        details_after.push('\n');
    }
    details_after.push_str(&format!("\n### §{}\n\n{}\n", slug, a.detail.trim()));
    optimisemd_core::write_atomic(&details, &details_after)?;

    let touched = vec![
        format!("docs/hard-won/{part_name}"),
        "docs/hard-won-details.md".to_string(),
    ];
    let check =
        optimisemd_core::util::run_script(&root.join("scripts/check-context-files.sh"), root);
    let ok = matches!(&check, Ok(o) if o.status.success());
    if !ok {
        optimisemd_core::write_atomic(&part, &part_before)?;
        optimisemd_core::write_atomic(&details, &details_before)?;
        // Say WHY the guard failed: a rolled-back save whose message hides the cause sends the next
        // session hunting blind.
        let (why, out) = match &check {
            Ok(o) => (
                format!("exit {}", o.status),
                String::from_utf8_lossy(&o.stdout).into_owned(),
            ),
            Err(e) => (format!("the guard could not run: {e}"), String::new()),
        };
        for l in out.lines().filter(|l| l.contains("FAIL")).take(5) {
            eprintln!("{l}");
        }
        let _ = optimisemd_core::audit::record(root, "landmine", "rolled-back", &touched);
        return Err(format!("the context guard failed ({why}); both files were restored").into());
    }
    let _ = optimisemd_core::audit::record(root, "landmine", "applied", &touched);
    println!("recorded §{slug} in docs/hard-won/{part_name}");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("optimisemd-landmine-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("docs/hard-won")).unwrap();
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::write(dir.join("docs/hard-won/01.md"), "# Traps\n").unwrap();
        fs::write(
            dir.join("docs/hard-won-details.md"),
            "# Landmine details\n\n### §taken-slug\n\nAlready recorded.\n",
        )
        .unwrap();
        fs::write(dir.join("MEMORY.md"), "# MEMORY.md\n").unwrap();
        let guard = dir.join("scripts/check-context-files.sh");
        fs::write(&guard, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&guard, fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    fn args<'a>(part: &'a str, text: &'a str, slug: &'a str) -> AddArgs<'a> {
        AddArgs {
            part,
            text,
            context: "the context",
            slug,
            detail: "the reproduction",
        }
    }

    #[test]
    fn a_part_cannot_escape_the_landmine_store() {
        let dir = fixture("escape");
        // ../../MEMORY.md exists through the join today — the line would land outside docs/hard-won.
        assert!(add(&dir, args("../../MEMORY.md", "A trap.", "a-slug")).is_err());
        assert!(!fs::read_to_string(dir.join("MEMORY.md"))
            .unwrap()
            .contains("A trap."));
        assert!(add(&dir, args("01.md/../../evil.md", "A trap.", "a-slug")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn line_fields_cannot_smuggle_rows_or_headings() {
        let dir = fixture("smuggle");
        assert!(add(
            &dir,
            args("01.md", "A trap.\n- **A smuggled row.**", "a-slug")
        )
        .is_err());
        assert!(add(&dir, args("01.md", "A trap.", "bad slug")).is_err());
        assert!(add(&dir, args("01.md", "A trap.", "a-slug\n### §another")).is_err());
        let part = fs::read_to_string(dir.join("docs/hard-won/01.md")).unwrap();
        assert!(!part.contains("smuggled"), "{part}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_slug_is_unique_across_the_details_too() {
        let dir = fixture("dupe");
        // §taken-slug lives in docs/hard-won-details.md but not in the part: adding it again would
        // create a second `### §taken-slug` heading in the details.
        assert!(add(&dir, args("01.md", "A trap.", "taken-slug")).is_err());
        let details = fs::read_to_string(dir.join("docs/hard-won-details.md")).unwrap();
        assert_eq!(details.matches("### §taken-slug").count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    // The success path runs a workspace shell script as its guard — a unix contract, like the
    // chmod and symlink tests elsewhere. Windows CI runs the refusal paths above.
    #[cfg(unix)]
    #[test]
    fn a_valid_landmine_lands_and_is_audited() {
        let dir = fixture("ok");
        assert_eq!(
            add(&dir, args("01.md", "A trap.", "fresh-slug")).unwrap(),
            0
        );
        assert!(fs::read_to_string(dir.join("docs/hard-won/01.md"))
            .unwrap()
            .contains("§fresh-slug"));
        assert!(fs::read_to_string(dir.join("memory/logs/repo-writes.log"))
            .unwrap()
            .contains("landmine applied"));
        let _ = fs::remove_dir_all(&dir);
    }
}
