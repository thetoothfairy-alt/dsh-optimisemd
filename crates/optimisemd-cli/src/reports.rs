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

pub(crate) fn local_datetime(epoch: i64) -> String {
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

/// The workspace's session folder name: the absolute path with slashes turned into dashes.
pub fn ws_key(root: &Path) -> String {
    format!(
        "--{}--",
        root.to_string_lossy()
            .trim_start_matches('/')
            .replace('/', "-")
    )
}

/// The root as the tool must see it: absolute. A relative root (".") turns the workspace's session folder into
/// "--.--", which is how the session index silently went empty from 0.4.0 to 0.4.4 (fixed 2026-11-16).
pub fn canonical_root(root: &Path) -> std::path::PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
}

/// Every transcript for this workspace, live first then archived, each list sorted by path.
pub fn transcript_files(home: &str, ws: &str) -> Vec<(String, std::path::PathBuf)> {
    let mut out = Vec::new();
    for (kind, base) in [
        ("live", format!("{home}/sessions/{ws}")),
        ("archived", format!("{home}/sessions-archive/{ws}")),
    ] {
        let Ok(entries) = fs::read_dir(&base) else {
            continue;
        };
        let mut logs: Vec<std::path::PathBuf> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| {
                let d = e.path();
                let z = d.join("session.v4.jsonl.zstd");
                if z.exists() {
                    z
                } else {
                    d.join("session.v4.jsonl")
                }
            })
            .filter(|p| p.exists())
            .collect();
        logs.sort();
        out.extend(logs.into_iter().map(|p| (kind.to_string(), p)));
    }
    out
}

/// Clip at n characters, with an ellipsis only when something was cut.
/// The transcript text: zstd when the file is compressed, plain text otherwise (older/uncompressed fixtures).
fn transcript_text(path: &Path) -> Option<String> {
    if path.to_string_lossy().ends_with(".zstd") {
        let out = Command::new("zstd")
            .args(["-dc", &path.to_string_lossy()])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        fs::read_to_string(path).ok()
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", take(s, n))
    } else {
        s.to_string()
    }
}

/// A question, cut before the harness's runtime reminders and background-job notices.
fn clean_ask(txt: &str) -> String {
    let mut cut = txt;
    for marker in [
        "<system-reminder>",
        "Current runtime context",
        "Time sampled while preparing",
        "Browser time zone for this request",
        "\n\nbackground job ",
        "Red/green discipline:",
        "Green gate:",
        "Your parent agent id is",
        "Delivery gate:",
        "Adversary review found:",
    ] {
        if let Some(i) = cut.find(marker) {
            cut = &cut[..i];
        }
    }
    cut.trim().to_string()
}

fn is_machine(subject: &str) -> bool {
    subject.starts_with("Auto backup") || subject.starts_with("nightly:")
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

/// The last title event wins: the harness writes a fallback title first, then the provider's title.
fn last_title(text: &str) -> String {
    let re = Regex::new(r#""type":"session/title[^"]*"[^\n]*?"title":"((?:[^"\\]|\\.)*)""#)
        .expect("constant regex");
    let mut title = String::new();
    for c in re.captures_iter(text) {
        title = unescape(&c[1]);
    }
    title
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
    let ws = ws_key(root);
    let out = root.join("docs/session-index.md");
    let created_re = Regex::new(r#""createdAt":(\d{13})"#)?;
    let text_re = Regex::new(r#""type":"text","text":"((?:[^"\\]|\\.){0,160})"#)?;
    let parent_re = Regex::new(r#""parentSession":"session-([0-9a-f-]+)""#)?;
    let mut rows: Vec<Row> = Vec::new();
    let files = transcript_files(&home, &ws);
    for (kind, log) in &files {
        let Some(text) = transcript_text(log) else {
            continue;
        };
        let text: String = text.chars().take(200_000).collect();
        let has_event = text.lines().any(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .map(|v| {
                    v.get("type")
                        .and_then(|t| t.as_str())
                        .map(|t| {
                            matches!(
                                t,
                                "session/title"
                                    | "session/title-set"
                                    | "session/start"
                                    | "request/header"
                                    | "user/message"
                                    | "agent/inbox/spliced"
                            )
                        })
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        });
        if !has_event {
            continue;
        }
        let sid = log
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| {
                n.to_string_lossy()
                    .trim_start_matches("session-")
                    .to_string()
            })
            .unwrap_or_default();
        let created_ms: Option<i64> = created_re
            .captures(&text)
            .and_then(|c| c[1].parse::<i64>().ok());
        let created: Option<i64> = created_ms.map(|ms| ms / 1000);
        let mut title = last_title(&text);
        if title.is_empty() {
            title = match text_re.captures(&text) {
                Some(c) => clip(&c[1], 120),
                None => "(untitled)".into(),
            };
        }
        let parent = parent_re
            .captures(&text)
            .map(|c| take(&c[1], 8))
            .unwrap_or_default();
        let epoch = match created {
            Some(s) => s,
            None => fs::metadata(log)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        };
        let size = fs::metadata(log).map(|m| m.len()).unwrap_or(0);
        rows.push(Row {
            sort_key: created_ms.unwrap_or(epoch * 1000),
            date: local_datetime(epoch),
            sid,
            parent,
            title: title.replace('\n', " "),
            kind: kind.clone(),
            size,
            lines: text.bytes().filter(|b| *b == b'\n').count(),
        });
    }
    if !check && rows.is_empty() {
        eprintln!(
            "session-index: no sessions found for this workspace under {home}/sessions/{ws} ({} transcript(s)); refusing to write an empty index",
            files.len()
        );
        return Ok(1);
    }
    rows.sort_by(|a, b| {
        b.sort_key
            .cmp(&a.sort_key)
            .then_with(|| b.sid.cmp(&a.sid))
            .then_with(|| b.title.cmp(&a.title))
    });

    if check {
        let newest = rows.first().map(|r| r.sort_key / 1000).unwrap_or(0);
        if !out.exists() {
            println!("MISSING — {} sessions, no index yet", rows.len());
            return Ok(1);
        }
        let out_mtime = fs::metadata(&out)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let stale = out_mtime < newest || rows.is_empty();
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
        let title = clip(&r.title, 110).replace('|', "\\|");
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
            "--date=format:%Y-%m-%d %H:%M",
            &format!("--format={fmt}"),
        ])
        .current_dir(root)
        .output()?;
    if !out.status.success() {
        eprintln!("session-commits: git log failed (not a repository?) — refusing to write an empty report");
        return Ok(1);
    }
    let raw = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut order: Vec<String> = Vec::new();
    let mut joined: HashMap<String, Vec<Commit>> = HashMap::new();
    let mut unjoined: Vec<Commit> = Vec::new();
    let mut machine: Vec<Commit> = Vec::new();
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
            if is_machine(&c.subject) {
                machine.push(c);
            } else {
                unjoined.push(c);
            }
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
    if titles.is_empty() {
        eprintln!(
            "session-commits: no titles read from docs/session-index.md — run session-index first (an empty index is not a blank-title report)"
        );
        return Ok(1);
    }
    let mut sessions: Vec<(&String, &Vec<Commit>)> =
        order.iter().map(|sid| (sid, &joined[sid])).collect();
    sessions.sort_by(|a, b| b.1.last().unwrap().date.cmp(&a.1.last().unwrap().date));
    let joined_commits: usize = joined.values().map(|v| v.len()).sum();
    let mut missing_titles = 0usize;
    for (sid, _) in &sessions {
        let short = take(&sid.replace("session-", ""), 8);
        if !titles.contains_key(&short) {
            missing_titles += 1;
        }
    }
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
            clip(
                titles
                    .get(&short)
                    .map(|s| s.as_str())
                    .unwrap_or("(not in the index)"),
                60
            ),
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
            clip(&c.subject, 90)
        ));
    }
    if unjoined.len() > 40 {
        lines.push(format!("- … and {} more", unjoined.len() - 40));
    }
    lines.push(String::new());
    if missing_titles > 0 {
        lines.push(format!(
            "Joined sessions with no title in docs/session-index.md: {missing_titles} — regenerate the index."
        ));
        lines.push(String::new());
    }
    lines.push(format!(
        "Machine commits with no session (auto backups, nightly runs): {}. They are not counted as unjoined.",
        machine.len()
    ));
    lines.push(String::new());
    lines.push(format!(
        "**{} commits in {} days · {} sessions joined · {} unjoined · {} machine.**",
        total,
        days,
        joined.len(),
        unjoined.len(),
        machine.len()
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
    let Ok(entries) = fs::read_dir(root.join("memory")) else {
        eprintln!("log-index: no memory/ directory — refusing to write an empty index");
        return Ok(1);
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if re_day.is_match(&name) {
            files.push(e.path());
        }
    }
    files.sort();
    let mut rows: Vec<(String, usize, usize, Vec<String>)> = Vec::new();
    for f in &files {
        let text = match fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "log-index: cannot read {} ({e}) — refusing to write a partial index",
                    f.display()
                );
                return Ok(1);
            }
        };
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
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    if rows.is_empty() {
        eprintln!("log-index: no dated daily logs — refusing to write an empty index");
        return Ok(1);
    }
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
    let ws = ws_key(root);
    let mut transcripts: Vec<std::path::PathBuf> = transcript_files(&home, &ws)
        .into_iter()
        .map(|(_, p)| p)
        .collect();
    transcripts.sort();
    let mut rows: Vec<(i64, String)> = Vec::new();
    for f in &transcripts {
        let Some(text) = transcript_text(f) else {
            continue;
        };
        let folder = f
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| {
                n.to_string_lossy()
                    .trim_start_matches("session-")
                    .to_string()
            })
            .unwrap_or_default();
        let sid = if folder.len() >= 8 && folder.chars().take(8).all(|c| c.is_ascii_hexdigit()) {
            take(&folder, 8)
        } else {
            folder.clone()
        };
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
            if t == "agent/inbox/spliced" {
                if let Some(items) = v
                    .get("data")
                    .and_then(|d| d.get("inserted"))
                    .and_then(|x| x.as_array())
                {
                    for item in items {
                        let source = item.get("source");
                        let kind = source
                            .and_then(|s| s.get("kind"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("");
                        let form = source
                            .and_then(|s| s.get("form"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("");
                        if kind == "dsh-doublecheck" || form == "notice" {
                            continue;
                        }
                        if let Some(blocks) = item.get("content").and_then(|c| c.as_array()) {
                            for b in blocks {
                                if b.get("type").and_then(|x| x.as_str()) != Some("text") {
                                    continue;
                                }
                                let txt = b.get("text").and_then(|x| x.as_str()).unwrap_or("");
                                if take(txt, 60).contains("memento") {
                                    continue;
                                }
                                let cleaned = clean_ask(txt);
                                let norm = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
                                if norm.chars().count() > 15 && !norm.starts_with("background job ")
                                {
                                    asks.push(take(&norm, 160));
                                }
                            }
                        }
                    }
                }
            }
            if t == "user/message" {
                let source = v.get("data").and_then(|d| d.get("source"));
                let kind = source
                    .and_then(|s| s.get("kind"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                let form = source
                    .and_then(|s| s.get("form"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                if kind == "dsh-doublecheck" || form == "notice" {
                    continue;
                }
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
                        if take(txt, 60).contains("memento") {
                            continue;
                        }
                        let cleaned = clean_ask(txt);
                        let norm = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
                        if norm.chars().count() > 15 && !norm.starts_with("background job ") {
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
    if rows.is_empty() {
        eprintln!(
            "session-digest: no sessions read ({} transcript(s)) — refusing to write an empty digest",
            transcripts.len()
        );
        return Ok(1);
    }
    fs::create_dir_all(root.join("memory/logs"))?;
    let mut body = String::from("# What past sessions were asked — one row per session\n\n");
    body.push_str("Generated by `bin/dsh-optimisemd session-digest`; the transcripts are this workspace's live and archived sessions (`~/.dsh/sessions/`, `sessions-archive/`). Indexed by `tools/belfind` so a question about past work can find the session that did it.\n\n");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn ws_key_is_the_workspace_folder_name() {
        assert_eq!(
            ws_key(Path::new("/home/admin/.openclaw/workspace")),
            "--home-admin-.openclaw-workspace--"
        );
    }

    #[test]
    fn canonical_root_makes_a_relative_root_absolute() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(canonical_root(Path::new(".")), cwd);
    }

    #[test]
    fn transcript_files_read_live_and_archive_for_this_workspace_only() {
        let home = std::env::temp_dir().join(format!("optimisemd-reports-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        for dir in [
            "sessions/--ws--/session-a",
            "sessions-archive/--ws--/session-b",
            "sessions/--other--/session-c",
        ] {
            fs::create_dir_all(home.join(dir)).unwrap();
            fs::write(home.join(dir).join("session.v4.jsonl.zstd"), b"").unwrap();
        }
        let got = transcript_files(&home.to_string_lossy(), "--ws--");
        let names: Vec<String> = got
            .iter()
            .map(|(_, p)| {
                p.parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert_eq!(names, vec!["session-a", "session-b"]);
        assert_eq!(got[0].0, "live");
        assert_eq!(got[1].0, "archived");
    }

    #[test]
    fn the_last_title_event_wins() {
        let text = "{\"type\":\"session/title\",\"data\":{\"title\":\"fallback cut\"}}\n\
                    {\"type\":\"session/title\",\"data\":{\"title\":\"The provider title\"}}\n";
        assert_eq!(last_title(text), "The provider title");
    }

    #[test]
    fn clip_adds_an_ellipsis_only_when_it_cuts() {
        assert_eq!(clip("abcdef", 3), "abc…");
        assert_eq!(clip("abc", 3), "abc");
    }

    #[test]
    fn the_ask_is_cut_before_harness_reminders() {
        assert_eq!(
            clean_ask("do the thing\n\nTime sampled while preparing turn 1: x"),
            "do the thing"
        );
        assert_eq!(clean_ask("do the thing"), "do the thing");
    }

    #[test]
    fn machine_commits_are_not_counted_as_unjoined_sessions() {
        assert!(is_machine("Auto backup 2026-11-11 03:00"));
        assert!(is_machine("nightly: refresh the measured counts"));
        assert!(!is_machine("Fix the portal"));
    }
}
