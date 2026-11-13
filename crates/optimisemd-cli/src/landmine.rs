//! Landmine creation: the one writing path that used to be a hand-run skill. It appends the trap line to its part, writes the
//! reproduction under `### §<slug>` in `docs/hard-won-details.md`, and runs the context guard as its gate — rolling both files
//! back when the guard fails, the same shape as `memory_save`'s checks gate.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;

pub struct AddArgs<'a> {
    pub part: &'a str,
    pub text: &'a str,
    pub context: &'a str,
    pub slug: &'a str,
    pub detail: &'a str,
}

const MAX_LINE: usize = 250;

pub fn add(root: &Path, a: AddArgs) -> Result<i32, Box<dyn Error>> {
    let part = root.join("docs/hard-won").join(a.part);
    let details = root.join("docs/hard-won-details.md");
    if !part.exists() {
        return Err(format!("no such part: {}", part.display()).into());
    }
    if !details.exists() {
        return Err("docs/hard-won-details.md is missing".into());
    }
    let line = format!("- **{}** — {} — §{}", a.text.trim(), a.context.trim(), a.slug.trim());
    if line.chars().count() > MAX_LINE {
        return Err(format!("line is {} characters; the cap is {MAX_LINE}", line.chars().count()).into());
    }
    let part_before = fs::read_to_string(&part)?;
    let details_before = fs::read_to_string(&details)?;
    if part_before.contains(&format!("§{}", a.slug.trim())) {
        return Err(format!("§{} already appears in that part", a.slug.trim()).into());
    }
    let mut part_after = part_before.clone();
    if !part_after.ends_with('\n') {
        part_after.push('\n');
    }
    part_after.push_str(&line);
    part_after.push('\n');
    fs::write(&part, &part_after)?;

    let mut details_after = details_before.clone();
    if !details_after.ends_with('\n') {
        details_after.push('\n');
    }
    details_after.push_str(&format!("\n### §{}\n\n{}\n", a.slug.trim(), a.detail.trim()));
    fs::write(&details, &details_after)?;

    let check = Command::new("bash")
        .arg(root.join("scripts/check-context-files.sh"))
        .current_dir(root)
        .output();
    let ok = matches!(&check, Ok(o) if o.status.success());
    if !ok {
        fs::write(&part, part_before)?;
        fs::write(&details, details_before)?;
        let out = check.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        for l in out.lines().filter(|l| l.contains("FAIL")).take(5) {
            eprintln!("{l}");
        }
        return Err("the context guard failed; both files were restored".into());
    }
    println!("recorded §{} in docs/hard-won/{}", a.slug.trim(), a.part);
    Ok(0)
}
