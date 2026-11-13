//! The four memory reports that used to be workspace scripts: session index, session → commits, session digest and the daily
//! log index. They read DSH session state and the workspace's own files and write one markdown report each, exactly like
//! `slim` and `export` do.

use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

pub fn local_now() -> String {
    Command::new("date")
        .arg("+%Y-%m-%d %H:%M")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn local_datetime(epoch: i64) -> String {
    Command::new("date")
        .args(["-d", &format!("@{epoch}"), "+%Y-%m-%d %H:%M"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn dsh_home() -> String {
    std::env::var("DSH_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/.dsh", std::env::var("HOME").unwrap_or_default()))
}

fn take(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/* ---------------------------------------------------------------- session index */

struct Row {
    sort_key: i64,
    date: String,
    sid: String,
    parent: String,
    title: String,
    kind: String,
    size: u64,
    lines: usize,
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('u') => {
                let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                if let Ok(code) = u32::from_str_radix(&hex, 16) {
                    if let Some(ch) = char::from_u32(code) {
                        out.push(ch);
                    }
                }
            }
            Some(other) => out.push(other),
            None => break,
        }
    }
    out
}

pub fn session_index(root: &Path, check: bool) -> Result<i32, Box<dyn std::error::Error>> {
    let home = dsh_home();
    let ws = format!(
        "--{}--",
        root.to_string_lossy()
            .trim_start_matches('/')
            .replace('/', "-")
    );
    let out = root.join("docs/session-index.md");
    let created_re = Regex::new(r#""createdAt":(\d{13})"#)?;
    let title_re = Regex::new(r#""type":"session/title"[^\n]*?"title":"((?:[^"\\]|\\.)*)""#)?;
    let text_re = Regex::new(r#""type":"text","text":"((?:[^"\\]|\\.){0,160})"#)?;
    let parent_re = Regex::new(r#""parentSession":"session-([0-9a-f-]+)""#)?;
    let mut rows: Vec<Row> = Vec::new();
    for (kind, base) in [
        ("live", format!("{home}/sessions/{ws}")),
        ("archived", format!("{home}/sessions-archive/{ws}")),
    ] {
        let entries = match fs::read_dir(&base) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let mut dirs: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .collect();
        dirs.sort();
        for d in dirs {
            let log = format!("{base}/{d}/session.v4.jsonl.zstd");
            if !Path::new(&log).exists() {
                continue;
            }
            let bytes = match Command::new("zstd").args(["-dc", &log]).output() {
                Ok(o) => o.stdout,
                Err(_) => continue,
            };
            let head = &bytes[..bytes.len().min(200_000)];
            let text = String::from_utf8_lossy(head).into_owned();
            let sid = d.trim_start_matches("session-").to_string();
            let created_ms: Option<i64> = created_re
                .captures(&text)
                .and_then(|c| c[1].parse::<i64>().ok());
            let created: Option<i64> = created_ms.map(|ms| ms / 1000);
            let mut title = match title_re.captures(&text) {
                Some(c) => unescape(&c[1]),
                None => String::new(),
            };
            if title.is_empty() {
                title = match text_re.captures(&text) {
                    Some(c) => format!("{}…", take(&c[1], 120)),
                    None => "(untitled)".into(),
                };
            }
            let parent = parent_re
                .captures(&text)
                .map(|c| take(&c[1], 8))
                .unwrap_or_default();
            let epoch = match created {
                Some(s) => s,
                None => fs::metadata(&log)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
            };
            let size = fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
            rows.push(Row {
                sort_key: created_ms.unwrap_or(epoch * 1000),
                date: local_datetime(epoch),
                sid,
                parent,
                title: title.replace('\n', " "),
                kind: kind.into(),
                size,
                lines: bytes.iter().filter(|b| **b == b'\n').count(),
            });
        }
    }
    rows.sort_by(|a, b| {
        b.sort_key
            .cmp(&a.sort_key)
            .then_with(|| b.sid.cmp(&a.sid))
            .then_with(|| b.title.cmp(&a.title))
    });

    if check && out.exists() {
        let newest = rows.first().map(|r| r.sort_key / 1000).unwrap_or(0);
        let out_mtime = fs::metadata(&out)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let stale = out_mtime < newest;
        println!(
            "{} — {} sessions, newest {}",
            if stale { "STALE" } else { "current" },
            rows.len(),
            rows.first().map(|r| r.date.clone()).unwrap_or_default()
        );
        return Ok(if stale { 1 } else { 0 });
    }

    let mut lines: Vec<String> = vec![
        "# Session index — every session in this workspace, newest first".into(), String::new(),
        format!("Generated by `bin/dsh-optimisemd session-index` at {}. The transcripts themselves are", local_now()),
        format!("`$DSH_HOME/sessions/{ws}/<id>/session.v4.jsonl.zstd` (Archived: `sessions-archive/`). This table is the index: date, id,"),
        "parent when it was a subagent, and the title the harness gave it. `--check` fails when a newer session exists than the".into(),
        "file records, so a stale index is visible rather than silent.".into(), String::new(),
        "| # | Started | Session | Parent | Title | Size | Lines |".into(), "|---|---|---|---|---|---|---|".into(),
    ];
    for (i, r) in rows.iter().enumerate() {
        let mark = if r.kind == "live" { "" } else { " (archived)" };
        let title = take(&r.title, 110).replace('|', "\\|");
        lines.push(format!(
            "| {} | {} | `{}`{} | {} | {} | {} KB | {} |",
            i + 1,
            r.date,
            take(&r.sid, 8),
            mark,
            if r.parent.is_empty() {
                String::new()
            } else {
                format!("`{}`", r.parent)
            },
            title,
            r.size / 1024,
            r.lines
        ));
    }
    let total: u64 = rows.iter().map(|r| r.size).sum();
    let archived = rows.iter().filter(|r| r.kind == "archived").count();
    lines.push(String::new());
    lines.push(format!(
        "**{} sessions** · {} MB of transcripts · {} archived.",
        rows.len(),
        total / 1024 / 1024,
        archived
    ));
    fs::write(&out, format!("{}\n", lines.join("\n")))?;
    println!(
        "wrote {}: {} sessions, {} MB",
        out.display(),
        rows.len(),
        total / 1024 / 1024
    );
    Ok(0)
}

/* ---------------------------------------------------------------- session → commits */

struct Commit {
    date: String,
    sha: String,
    subject: String,
}

pub fn session_commits(root: &Path, days: &str) -> Result<i32, Box<dyn std::error::Error>> {
    let fmt = "%H\u{1f}%ad\u{1f}%s\u{1f}%(trailers:key=Dsh-Session,valueonly)%x1e".to_string();
    let out = Command::new("git")
        .args([
            "log",
            &format!("--since={days} days ago"),
            &format!("--format={fmt}"),
        ])
        .current_dir(root)
        .output()?;
    let raw = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut order: Vec<String> = Vec::new();
    let mut joined: HashMap<String, Vec<Commit>> = HashMap::new();
    let mut unjoined: Vec<Commit> = Vec::new();
    let mut total = 0usize;
    for rec in raw.split('\u{1e}') {
        let rec = rec.trim_matches('\n');
        let parts: Vec<&str> = rec.split('\u{1f}').collect();
        if parts.len() < 4 || parts[0].is_empty() {
            continue;
        }
        total += 1;
        let c = Commit {
            sha: parts[0].into(),
            date: parts[1].into(),
            subject: parts[2].into(),
        };
        let sid = parts[3]
            .trim()
            .split('\n')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if sid.is_empty() {
            unjoined.push(c);
        } else {
            if !joined.contains_key(&sid) {
                order.push(sid.clone());
                joined.insert(sid.clone(), Vec::new());
            }
            joined.get_mut(&sid).unwrap().push(c);
        }
    }
    let mut titles: HashMap<String, String> = HashMap::new();
    let index = root.join("docs/session-index.md");
    if index.exists() {
        if let Ok(text) = fs::read_to_string(&index) {
            let re = Regex::new(
                r"\| [\d\u{2013}-]+ \| ([\d-]+ [\d:]+) \| `([0-9a-f]{8})`[^|]*\|[^|]*\| ([^|]*) \|",
            )?;
            for caps in re.captures_iter(&text) {
                titles.insert(caps[2].to_string(), caps[3].trim().to_string());
            }
        }
    }
    let mut sessions: Vec<(&String, &Vec<Commit>)> =
        order.iter().map(|sid| (sid, &joined[sid])).collect();
    sessions.sort_by(|a, b| b.1.last().unwrap().date.cmp(&a.1.last().unwrap().date));
    let joined_commits: usize = joined.values().map(|v| v.len()).sum();
    let mut lines: Vec<String> = vec![
        "# Sessions → commits — the join between the transcript and the repository".into(), String::new(),
        format!("Generated by `bin/dsh-optimisemd session-commits` at {} over the last {} days.", local_now(), days),
        "The `Dsh-Session:` trailer is written automatically by `scripts/commit-msg-check.sh`; before 2026-10-27 nothing recorded".into(),
        "it, so older commits cannot be joined. Read a session title here, then open its transcript via `docs/session-index.md` —".into(),
        "or run `git log --grep=\"Dsh-Session: <id>\"` to see exactly what that session changed.".into(), String::new(),
        format!("## Joined — {} commits across {} sessions", joined_commits, joined.len()), String::new(),
        "| Session | Title | Commits | First | Last |".into(), "|---|---|---|---|---|".into(),
    ];
    for (sid, rows) in &sessions {
        let short = take(&sid.replace("session-", ""), 8);
        let mut dates: Vec<&String> = rows.iter().map(|r| &r.date).collect();
        dates.sort();
        lines.push(format!(
            "| `{}` | {} | {} | {} | {} |",
            short,
            take(titles.get(&short).map(|s| s.as_str()).unwrap_or(""), 60),
            rows.len(),
            take(dates[0], 16),
            take(dates[dates.len() - 1], 16)
        ));
    }
    lines.push(String::new());
    lines.push(format!(
        "## Not joined — {} commits with no trailer (pre-hook, or another session's tool)",
        unjoined.len()
    ));
    lines.push(String::new());
    for c in unjoined.iter().take(40) {
        lines.push(format!(
            "- {} `{}` {}",
            take(&c.date, 10),
            take(&c.sha, 8),
            take(&c.subject, 90)
        ));
    }
    if unjoined.len() > 40 {
        lines.push(format!("- … and {} more", unjoined.len() - 40));
    }
    lines.push(String::new());
    lines.push(format!(
        "**{} commits in {} days · {} sessions joined · {} unjoined.**",
        total,
        days,
        joined.len(),
        unjoined.len()
    ));
    fs::write(
        root.join("docs/session-commits.md"),
        format!("{}\n", lines.join("\n")),
    )?;
    println!(
        "wrote docs/session-commits.md: {} commits, {} sessions joined, {} unjoined",
        total,
        joined.len(),
        unjoined.len()
    );
    Ok(0)
}

/* ---------------------------------------------------------------- daily log index */

pub fn memory_log_index(root: &Path) -> Result<i32, Box<dyn std::error::Error>> {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let re_day = Regex::new(r"^\d{4}-\d{2}-\d{2}\.md$")?;
    if let Ok(entries) = fs::read_dir(root.join("memory")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if re_day.is_match(&name) {
                files.push(e.path());
            }
        }
    }
    files.sort();
    let mut rows: Vec<(String, usize, usize, Vec<String>)> = Vec::new();
    for f in &files {
        let text = fs::read_to_string(f).unwrap_or_default();
        let h2: Vec<String> = text
            .lines()
            .filter_map(|l| {
                l.strip_prefix("## ")
                    .map(|s| s.trim().trim_matches('#').trim().to_string())
            })
            .collect();
        let mut heads = h2;
        if heads.is_empty() {
            heads = text
                .lines()
                .filter_map(|l| {
                    l.strip_prefix("### ")
                        .map(|s| s.trim().trim_matches('#').trim().to_string())
                })
                .collect();
        }
        heads.retain(|h| !h.to_ascii_lowercase().starts_with("openclaw:"));
        let day = f.file_name().unwrap().to_string_lossy()[..10].to_string();
        rows.push((day, text.chars().count(), text.lines().count(), heads));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    let mut lines: Vec<String> = vec![
        "# Daily log index — one line per day, with that day's topics".into(), String::new(),
        format!("Generated by `bin/dsh-optimisemd log-index` at {}. The logs are", local_now()),
        "`memory/YYYY-MM-DD.md`; this is the table of contents to them. Topics are the files' own `##` headings, so they cost".into(),
        "nothing to maintain — write a heading when you write the section, and the index stays true.".into(), String::new(),
        "| Day | Size | Lines | Topics |".into(), "|---|---|---|---|".into(),
    ];
    let mut total = 0usize;
    for (day, size, nlines, heads) in &rows {
        total += size;
        let mut topics = heads
            .iter()
            .take(8)
            .map(|h| take(h, 60).replace('|', "\\|"))
            .collect::<Vec<_>>()
            .join(" · ");
        if heads.len() > 8 {
            topics.push_str(&format!(" · … +{}", heads.len() - 8));
        }
        lines.push(format!(
            "| {} | {} KB | {} | {} |",
            day,
            size / 1024,
            nlines,
            topics
        ));
    }
    lines.push(String::new());
    lines.push(format!("**{} days · {} MB of logs.** A day with no headings says so by having none — that is the signal to go and read it.",
        rows.len(), total / 1024 / 1024));
    fs::write(
        root.join("docs/memory-log-index.md"),
        format!("{}\n", lines.join("\n")),
    )?;
    println!(
        "wrote docs/memory-log-index.md: {} days, {} MB",
        rows.len(),
        total / 1024 / 1024
    );
    Ok(0)
}

/* ---------------------------------------------------------------- session digest */

pub fn session_digest(root: &Path) -> Result<i32, Box<dyn std::error::Error>> {
    let home = dsh_home();
    let mut transcripts: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(outer) = fs::read_dir(format!("{home}/sessions")) {
        for w in outer.flatten() {
            if !w.path().is_dir() {
                continue;
            }
            if let Ok(inner) = fs::read_dir(w.path()) {
                for s in inner.flatten() {
                    let name = s.file_name().to_string_lossy().to_string();
                    let log = s.path().join("session.v4.jsonl.zstd");
                    if name.starts_with("session-") && log.exists() {
                        transcripts.push(log);
                    }
                }
            }
        }
    }
    transcripts.sort();
    let mut rows: Vec<(i64, String)> = Vec::new();
    for f in &transcripts {
        let bytes = match Command::new("zstd")
            .args(["-dc", &f.to_string_lossy()])
            .output()
        {
            Ok(o) => o.stdout,
            Err(_) => continue,
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let path_s = f.to_string_lossy().into_owned();
        let sid = path_s
            .split("/session-")
            .last()
            .unwrap_or("")
            .split('/')
            .next()
            .unwrap_or("");
        let sid = take(sid, 8);
        let mut title = String::new();
        let mut created: Option<i64> = None;
        let mut asks: Vec<String> = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
            if created.is_none() {
                if let Some(c) = v.get("createdAt").and_then(|x| x.as_i64()) {
                    created = Some(c);
                }
            }
            let data_title = v
                .get("data")
                .and_then(|d| d.get("title"))
                .and_then(|x| x.as_str());
            if t == "session/title"
                || t == "session/title-set"
                || (t.contains("title") && data_title.is_some())
            {
                if let Some(dt) = data_title {
                    title = dt.to_string();
                }
            }
            if t == "user/message" {
                if let Some(blocks) = v
                    .get("data")
                    .and_then(|d| d.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for b in blocks {
                        if b.get("type").and_then(|x| x.as_str()) != Some("text") {
                            continue;
                        }
                        let txt = b.get("text").and_then(|x| x.as_str()).unwrap_or("");
                        if txt.contains("<system-reminder>")
                            || txt.contains("Current runtime context")
                            || take(txt, 60).contains("memento")
                        {
                            continue;
                        }
                        let norm = txt.split_whitespace().collect::<Vec<_>>().join(" ");
                        if norm.chars().count() > 15 {
                            asks.push(take(&norm, 160));
                        }
                    }
                }
            }
        }
        if asks.is_empty() && title.is_empty() {
            continue;
        }
        let when = match created {
            Some(c) => local_datetime_short(c / 1000),
            None => "?".into(),
        };
        let shown: Vec<String> = if asks.is_empty() {
            Vec::new()
        } else {
            let mut out = vec![asks[0].clone()];
            for a in asks.iter().skip(1).take(5) {
                if a != &asks[0] && out.len() < 3 {
                    out.push(a.clone());
                }
            }
            out
        };
        let mut row = format!(
            "- **{} · {} · {}**",
            when,
            sid,
            if title.is_empty() {
                "(untitled)"
            } else {
                &title
            }
        );
        if !shown.is_empty() {
            row.push_str(&format!(" — asked: {}", shown.join(" | ")));
        }
        rows.push((created.unwrap_or(0), row));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    fs::create_dir_all(root.join("memory/logs"))?;
    let mut body = String::from("# What past sessions were asked — one row per session\n\n");
    body.push_str("Generated by `bin/dsh-optimisemd session-digest`; the transcripts themselves are `~/.dsh/sessions/`. Indexed by `tools/belfind` so a question about past work can find the session that did it.\n\n");
    for (_, r) in &rows {
        body.push_str(r);
        body.push('\n');
    }
    fs::write(root.join("memory/logs/session-digest.md"), body)?;
    println!(
        "wrote memory/logs/session-digest.md: {} sessions",
        rows.len()
    );
    Ok(0)
}

fn local_datetime_short(epoch: i64) -> String {
    Command::new("date")
        .args(["-d", &format!("@{epoch}"), "+%Y-%m-%d %H:%M"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}
