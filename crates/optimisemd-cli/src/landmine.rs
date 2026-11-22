//! Landmine creation: the one writing path that used to be a hand-run skill. It appends the trap line to its part, writes the
//! reproduction under `### §<slug>` in `docs/hard-won-details/<part>.md` with its slug row in the `docs/hard-won-details.md`
//! index, and runs the context guard as its gate — rolling every touched file back when the guard fails, the same shape as
//! `memory_save`'s checks gate.

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

/// The continuation part of `NN-<subject>.md`: `MM-<subject>-<k+1>.md` with the next free number and
/// the next free subject counter (`05-web-and-onedrive.md` → `09-web-and-onedrive-3.md` when 08 already
/// continues it). The tool names it so a writer never has to, and never has to refuse a save.
fn continuation(part_name: &str, dir: &Path) -> Option<String> {
    let stem = part_name.strip_suffix(".md")?;
    let (num, desc) = stem.split_once('-')?;
    if !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (base, k) = match desc.rsplit_once('-') {
        Some((b, s)) if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) => {
            (b.to_string(), s.parse::<u32>().ok()?)
        }
        _ => (desc.to_string(), 1u32),
    };
    let mut maxk = k;
    let mut maxn = num.parse::<u32>().ok()?;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let Some(stem) = n.strip_suffix(".md") else {
                continue;
            };
            let Some((pfx, d)) = stem.split_once('-') else {
                continue;
            };
            if let Ok(v) = pfx.parse::<u32>() {
                maxn = maxn.max(v);
            }
            let kk = if d == base {
                Some(1)
            } else {
                d.strip_prefix(&format!("{base}-")).and_then(|s| {
                    if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
                        s.parse::<u32>().ok()
                    } else {
                        None
                    }
                })
            };
            if let Some(kk) = kk {
                maxk = maxk.max(kk);
            }
        }
    }
    Some(format!("{:02}-{}-{}.md", maxn + 1, base, maxk + 1))
}

/// The length of the `§` slug at the start of `s`: letters, digits, dashes, underscores and dots,
/// the same shape the guard's rule 15 reads. Exact addresses only — `§a-slug` must not claim `§a-slug-2`.
fn slug_end(s: &str) -> usize {
    s.find(|c: char| {
        !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.')
    })
    .unwrap_or(s.len())
}

/// The `§` address a heading carries, if any: `#{2,} §slug`, with or without a description after it.
fn heading_slug(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if hashes < 2 {
        return None;
    }
    let rest = t[hashes..].trim_start().strip_prefix('§')?;
    Some(&rest[..slug_end(rest)])
}

/// Whether any details file already holds a reproduction section for this exact slug.
fn section_recorded(dir: &Path, slug: &str) -> bool {
    let Ok(rd) = fs::read_dir(dir) else {
        return false;
    };
    rd.flatten().any(|e| {
        let p = dir.join(e.file_name());
        p.extension().map(|x| x == "md").unwrap_or(false)
            && fs::read_to_string(&p)
                .unwrap_or_default()
                .lines()
                .any(|l| heading_slug(l) == Some(slug))
    })
}

/// Whether an index row already points at this exact slug (`| §slug | \`file\` |`).
fn row_is_for(line: &str, slug: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix("| §") else {
        return false;
    };
    let end = slug_end(rest);
    rest[..end] == *slug && rest[end..].starts_with(" |")
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
    let details_dir = root.join("docs/hard-won-details");
    let details_index = root.join("docs/hard-won-details.md");
    if !part.exists() {
        return Err(format!("no such part: {}", part.display()).into());
    }
    if !details_index.exists() {
        return Err("docs/hard-won-details.md (the slug index) is missing".into());
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
    let index_before = fs::read_to_string(&details_index)?;
    // The slug is unique across the whole store: a second `§` address anywhere is a duplicate
    // whichever part the line lives in. Exact addresses only — `§a-slug` must not claim `§a-slug-2`.
    // A hand-deleted index row opens no door: the reproduction section itself is the record.
    let dup = section_recorded(&details_dir, slug)
        || index_before.lines().any(|l| row_is_for(l, slug))
        || fs::read_dir(root.join("docs/hard-won"))
            .map(|rd| {
                rd.flatten().any(|e| {
                    let p = root.join("docs/hard-won").join(e.file_name());
                    p.extension().map(|x| x == "md").unwrap_or(false)
                        && fs::read_to_string(&p)
                            .unwrap_or_default()
                            .lines()
                            .any(|l| l.ends_with(&format!("§{slug}")))
                })
            })
            .unwrap_or(false);
    if dup {
        return Err(format!("§{slug} is already recorded").into());
    }

    // (path, content after, content before — None when this call creates the file)
    let mut writes: Vec<(std::path::PathBuf, String, Option<String>)> = Vec::new();
    let target_rel = format!("docs/hard-won/{part_name}");
    let mut created: Option<String> = None;
    // The part file the line lands in — the reproduction gets the details file named after it.
    let mut landed = part_name.to_string();

    let budget = crate::slim::budget_for(
        &fs::read_to_string(root.join("scripts/check-context-files.sh")).unwrap_or_default(),
        &target_rel,
    );
    if crate::slim::crosses_budget(&part_before, &format!("{line}\n"), budget) {
        // The part is full: close it byte-for-byte and continue in the next part for the same
        // subject. The tool creates the part and its table row, so no save is ever refused.
        let index = root.join("docs/hard-won.md");
        let index_before = fs::read_to_string(&index).map_err(|_| {
            "docs/hard-won.md is missing — the parts table is what keeps a rotation findable"
        })?;
        let next = continuation(part_name, &root.join("docs/hard-won")).ok_or(
            "this part's name carries no NN-subject number, so the next part cannot be named",
        )?;
        let marker = format!("| `docs/hard-won/{part_name}` |");
        let mut at = None;
        let mut desc = String::new();
        for (i, l) in index_before.lines().enumerate() {
            if l.starts_with(&marker) {
                at = Some(i);
                let cells: Vec<&str> = l.split('|').collect();
                if cells.len() >= 3 {
                    desc = cells[2].trim().to_string();
                }
                break;
            }
        }
        let at = at.ok_or(
            "docs/hard-won.md has no row for the closing part — add its row before rotating",
        )?;
        // A continuation row keeps only the subject's own description.
        let desc = desc
            .rsplit_once("(closed at budget): ")
            .map(|(_, d)| d.to_string())
            .unwrap_or(desc);
        let num = part_name.split('-').next().unwrap_or("");
        let row =
            format!("| `docs/hard-won/{next}` | continues {num} (closed at budget): {desc} |");
        let mut index_after = String::new();
        for (i, l) in index_before.lines().enumerate() {
            index_after.push_str(l);
            index_after.push('\n');
            if i == at {
                index_after.push_str(&row);
                index_after.push('\n');
            }
        }
        writes.push((index, index_after, Some(index_before)));
        writes.push((
            root.join("docs/hard-won").join(&next),
            format!("# Landmines — {desc}\n\n{line}\n"),
            None,
        ));
        landed = next.clone();
        created = Some(next);
    } else {
        let mut part_after = part_before.clone();
        if !part_after.ends_with('\n') {
            part_after.push('\n');
        }
        part_after.push_str(&line);
        part_after.push('\n');
        writes.push((part.clone(), part_after, Some(part_before.clone())));
    }

    // The reproduction lives beside its line: one details file per part, named after the part, and
    // the old single file stays as the slug index so every citation of it keeps landing somewhere.
    let details_file = details_dir.join(&landed);
    let details_file_before = fs::read_to_string(&details_file).ok();
    let mut details_after = details_file_before.clone().unwrap_or_default();
    if details_file_before.is_none() {
        details_after.push_str(&format!(
            "# Hard-won details — the long entries behind the lines in docs/hard-won/{landed}\n"
        ));
    }
    if !details_after.ends_with('\n') {
        details_after.push('\n');
    }
    details_after.push_str(&format!("\n### §{}\n\n{}\n", slug, a.detail.trim()));
    writes.push((details_file, details_after, details_file_before));
    // One index row per slug names the file that carries the section.
    let row = format!("| §{} | `docs/hard-won-details/{}` |", slug, landed);
    let mut index_after = index_before.clone();
    if !index_after.ends_with('\n') {
        index_after.push('\n');
    }
    index_after.push_str(&row);
    index_after.push('\n');
    writes.push((
        details_index.clone(),
        index_after,
        Some(index_before.clone()),
    ));

    // One transaction over every touched file: a landmine whose entry landed without its
    // reproduction (or the reverse) is a half-recorded trap.
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
    let check =
        optimisemd_core::util::run_script(&root.join("scripts/check-context-files.sh"), root);
    let ok = matches!(&check, Ok(o) if o.status.success());
    if !ok {
        for (p, _, before) in &writes {
            match before {
                Some(b) => optimisemd_core::write_atomic(p, b)?,
                None => {
                    let _ = fs::remove_file(p);
                }
            }
        }
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
        return Err(format!("the context guard failed ({why}); every file was restored").into());
    }
    let _ = optimisemd_core::audit::record(root, "landmine", "applied", &touched);
    match &created {
        Some(next) => println!(
            "recorded §{slug} in docs/hard-won/{next} — {part_name} closed at budget and stays byte-for-byte"
        ),
        None => println!("recorded §{slug} in docs/hard-won/{part_name}"),
    }
    Ok(0)
}

#[cfg(test)]
mod tests {

    /// The gate and the finder are shell scripts: a host with no POSIX shell cannot run them. Instead of
    /// compiling these tests out, say so out loud — and fail when REQUIRE_SHELL=1 (CI sets it, so a
    /// runner that loses its shell is a red run, not a silent skip).
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
            std::env::temp_dir().join(format!("optimisemd-landmine-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("docs/hard-won")).unwrap();
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::write(dir.join("docs/hard-won/01.md"), "# Traps\n").unwrap();
        fs::create_dir_all(dir.join("docs/hard-won-details")).unwrap();
        fs::write(
            dir.join("docs/hard-won-details.md"),
            "# Landmine details — the index\n\n| §slug | reproduction |\n|---|---|\n| §taken-slug | `docs/hard-won-details/01.md` |\n",
        )
        .unwrap();
        fs::write(
            dir.join("docs/hard-won-details/01.md"),
            "# Details — 01\n\n### §taken-slug\n\nAlready recorded.\n",
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
        // §taken-slug has a reproduction in docs/hard-won-details/01.md but no line in the part:
        // adding it again would create a second `### §taken-slug` heading in the details family.
        assert!(add(&dir, args("01.md", "A trap.", "taken-slug")).is_err());
        let details = fs::read_to_string(dir.join("docs/hard-won-details/01.md")).unwrap();
        assert_eq!(details.matches("### §taken-slug").count(), 1);
        // A hand-deleted index row must not open the door either: the section itself is the record.
        let index = fs::read_to_string(dir.join("docs/hard-won-details.md")).unwrap();
        fs::write(
            dir.join("docs/hard-won-details.md"),
            index.replace("| §taken-slug | `docs/hard-won-details/01.md` |", ""),
        )
        .unwrap();
        assert!(add(&dir, args("01.md", "A trap.", "taken-slug")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    // The success path runs a workspace shell script as its guard: tested wherever a POSIX shell
    // exists, announced (and on CI, failed) where there is none (v0.4.9).
    #[test]
    fn a_valid_landmine_lands_and_is_audited() {
        if !optimisemd_core::util::shell_available() {
            return no_shell("a landmine landing through the gate");
        }
        let dir = fixture("ok");
        assert_eq!(
            add(&dir, args("01.md", "A trap.", "fresh-slug")).unwrap(),
            0
        );
        assert!(fs::read_to_string(dir.join("docs/hard-won/01.md"))
            .unwrap()
            .contains("§fresh-slug"));
        // The reproduction lives in the details file for its part, and the index names it — the
        // index is a map, never a second copy of the text.
        assert!(
            fs::read_to_string(dir.join("docs/hard-won-details/01.md"))
                .unwrap()
                .contains("### §fresh-slug"),
            "the reproduction lands in docs/hard-won-details/01.md"
        );
        let map = fs::read_to_string(dir.join("docs/hard-won-details.md")).unwrap();
        assert!(
            map.contains("| §fresh-slug | `docs/hard-won-details/01.md` |"),
            "{map}"
        );
        assert!(
            !map.contains("the reproduction"),
            "the reproduction body must not be copied into the index"
        );
        assert!(fs::read_to_string(dir.join("memory/logs/repo-writes.log"))
            .unwrap()
            .contains("landmine applied"));
        let _ = fs::remove_dir_all(&dir);
    }

    // The redesign's promise: a full part is a routine transition, never a refused save. The tool
    // opens the continuation part and its table row itself, and the closed part stays byte-for-byte.
    #[test]
    fn a_full_part_continues_in_a_new_part_and_stays_byte_for_byte() {
        if !optimisemd_core::util::shell_available() {
            return no_shell("a landmine rotating to the continuation part");
        }
        let dir = fixture("rotate");
        fs::write(
            dir.join("docs/hard-won.md"),
            "# Hard-won — the index\n\n| Part | What belongs there |\n|---|---|\n| `docs/hard-won/01-claims.md` | a subject |\n",
        )
        .unwrap();
        fs::write(dir.join("docs/hard-won/01.md"), "").unwrap();
        let _ = fs::remove_file(dir.join("docs/hard-won/01.md"));
        fs::write(
            dir.join("docs/hard-won/01-claims.md"),
            "# Traps\n\n- **aaaa.** bbbb — §old\n",
        )
        .unwrap();
        // The budget home the writer and the gate read together: one more line crosses it.
        let guard = dir.join("scripts/check-context-files.sh");
        fs::write(
            &guard,
            "#!/usr/bin/env bash\nexit 0\nBUDGETS_BLOCK=$(cat <<'BUDGETS'\n('docs/hard-won/*.md', 60, 60),\nBUDGETS\n)\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&guard, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let before = fs::read_to_string(dir.join("docs/hard-won/01-claims.md")).unwrap();
        assert_eq!(
            add(&dir, args("01-claims.md", "A trap.", "fresh-slug")).unwrap(),
            0
        );
        assert_eq!(
            fs::read_to_string(dir.join("docs/hard-won/01-claims.md")).unwrap(),
            before,
            "the closed part stays byte-for-byte"
        );
        assert!(
            fs::read_to_string(dir.join("docs/hard-won/02-claims-2.md"))
                .unwrap()
                .contains("§fresh-slug"),
            "the landmine lands in the continuation part"
        );
        assert!(
            fs::read_to_string(dir.join("docs/hard-won-details/02-claims-2.md"))
                .unwrap()
                .contains("### §fresh-slug"),
            "the reproduction follows the line into the continuation's details file"
        );
        let map = fs::read_to_string(dir.join("docs/hard-won-details.md")).unwrap();
        assert!(
            map.contains("| §fresh-slug | `docs/hard-won-details/02-claims-2.md` |"),
            "{map}"
        );
        let index = fs::read_to_string(dir.join("docs/hard-won.md")).unwrap();
        assert!(
            index.contains(
                "| `docs/hard-won/02-claims-2.md` | continues 01 (closed at budget): a subject |"
            ),
            "{index}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
