use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

/// Caps per track/scope pair, in characters of entry text. The published budgets were user 2000 and
/// agent 4000; they live in code so a config mistake cannot lift them silently.
#[derive(Debug, Clone, Copy)]
pub struct Budgets {
    pub user: usize,
    pub agent: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            user: 2000,
            agent: 4000,
        }
    }
}

/// One row of the store's audit table: what was asked, when, and by which session.
#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub seq: i64,
    pub ts: i64,
    pub action: String,
    pub track: String,
    pub scope: String,
    pub entry_id: String,
    pub text: String,
    pub outcome: String,
    pub source: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: String,
    pub track: String,
    pub scope: String,
    pub workspace_key: String,
    pub agent_key: String,
    pub text: String,
    pub tags: Vec<String>,
    pub source: String,
    pub created_at: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("store: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not found: no entry contains {0:?}")]
    NotFound(String),
    #[error("ambiguous: {0} entries contain the match")]
    Ambiguous(usize),
    #[error("budget: {track}/{scope} would reach {after} of {cap} characters; consolidate first")]
    Budget {
        track: String,
        scope: String,
        after: usize,
        cap: usize,
    },
}

pub struct Store {
    conn: Connection,
    budgets: Budgets,
}

/// A write transaction that holds the database's write lock from the first statement (BEGIN IMMEDIATE),
/// so the cap check and the insert it guards cannot interleave with another writer. Rolls back on drop
/// unless `commit` was reached.
struct Tx<'c> {
    conn: &'c Connection,
    active: bool,
}

impl<'c> Tx<'c> {
    fn begin(conn: &'c Connection) -> Result<Self, StoreError> {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(Self { conn, active: true })
    }

    fn commit(mut self) -> Result<(), StoreError> {
        self.conn.execute_batch("COMMIT")?;
        self.active = false;
        Ok(())
    }
}

impl Drop for Tx<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

impl Store {
    pub fn open(path: &Path, budgets: Budgets) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Two sessions writing at once must queue, not fail instantly on SQLITE_BUSY.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "create table if not exists entries (
                 id text primary key,
                 track text not null,
                 scope text not null,
                 workspace_key text not null default '',
                 agent_key text not null default '',
                 text text not null,
                 tags text not null default '[]',
                 source text not null default 'memory-tool',
                 created_at integer not null
             );
             create index if not exists entries_track_scope on entries(track, scope);
             create table if not exists audit (
                 seq integer primary key autoincrement,
                 ts integer not null,
                 action text not null,
                 track text not null,
                 scope text not null,
                 entry_id text not null default '',
                 text text not null default '',
                 outcome text not null default '',
                 source text not null default '',
                 session_id text not null default ''
             );",
        )?;
        Ok(Self { conn, budgets })
    }

    /// A read-only handle for the read paths (list, query, verify, history, export): it creates
    /// nothing and writes nothing — reading the legacy dsh-memento store must not touch it.
    pub fn open_readonly(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Self {
            conn,
            budgets: Budgets::default(),
        })
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    // Nine named fields on one audit row is the shape the plugin's table has; grouping them into a struct would only
    // move the same list to the call site.
    #[allow(clippy::too_many_arguments)]
    fn audit(
        &self,
        action: &str,
        track: &str,
        scope: &str,
        id: &str,
        text: &str,
        outcome: &str,
        source: &str,
        session: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "insert into audit (ts,action,track,scope,entry_id,text,outcome,source,session_id) values (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![Self::now_ms(), action, track, scope, id, text, outcome, source, session],
        )?;
        Ok(())
    }

    pub fn usage(&self, track: &str, scope: &str) -> Result<usize, StoreError> {
        let used: i64 = self.conn.query_row(
            "select coalesce(sum(length(text)),0) from entries where track=?1 and scope=?2",
            rusqlite::params![track, scope],
            |r| r.get(0),
        )?;
        Ok(used as usize)
    }

    /// The audit table, newest first: every add, replace, remove, query, merge and import the store recorded.
    pub fn history(&self, limit: usize) -> Result<Vec<HistoryRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "select seq,ts,action,track,scope,entry_id,text,outcome,source,session_id from audit \
             order by seq desc limit ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit.max(1) as i64], |r| {
            Ok(HistoryRow {
                seq: r.get(0)?,
                ts: r.get(1)?,
                action: r.get(2)?,
                track: r.get(3)?,
                scope: r.get(4)?,
                entry_id: r.get(5)?,
                text: r.get(6)?,
                outcome: r.get(7)?,
                source: r.get(8)?,
                session_id: r.get(9)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn cap(&self, track: &str) -> usize {
        match track {
            "user" => self.budgets.user,
            _ => self.budgets.agent,
        }
    }

    pub fn list_entries(&self) -> Result<Vec<Entry>, StoreError> {
        let mut stmt = self.conn.prepare(
            "select id,track,scope,workspace_key,agent_key,text,tags,source,created_at from entries order by created_at, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Entry {
                id: r.get(0)?,
                track: r.get(1)?,
                scope: r.get(2)?,
                workspace_key: r.get(3)?,
                agent_key: r.get(4)?,
                text: r.get(5)?,
                tags: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                source: r.get(7)?,
                created_at: r.get(8)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &self,
        track: &str,
        scope: &str,
        text: &str,
        tags: &[String],
        source: &str,
        workspace_key: &str,
        agent_key: &str,
        session: &str,
    ) -> Result<String, StoreError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(StoreError::Invalid("text is empty".into()));
        }
        if text.contains('\n') || text.contains('\r') {
            return Err(StoreError::Invalid(
                "entry text must be one line — no newline".into(),
            ));
        }
        if track != "user" && track != "agent" {
            return Err(StoreError::Invalid(format!("unknown track {track:?}")));
        }
        if scope != "user-global" && scope != "workspace" {
            return Err(StoreError::Invalid(format!("unknown scope {scope:?}")));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".into());
        // The cap check and the insert move together: two sessions writing at once must not both
        // pass the check and jointly exceed the budget.
        let tx = Tx::begin(&self.conn)?;
        let after = self.usage(track, scope)? + text.chars().count();
        let cap = self.cap(track);
        if after > cap {
            return Err(StoreError::Budget {
                track: track.into(),
                scope: scope.into(),
                after,
                cap,
            });
        }
        self.conn.execute(
            "insert into entries (id,track,scope,workspace_key,agent_key,text,tags,source,created_at) values (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![id, track, scope, workspace_key, agent_key, text, tags_json, source, Self::now_ms()],
        )?;
        self.audit("add", track, scope, &id, text, "applied", source, session)?;
        tx.commit()?;
        Ok(id)
    }

    fn find_matches(&self, needle: &str) -> Result<Vec<Entry>, StoreError> {
        let all = self.list_entries()?;
        let lower = needle.to_lowercase();
        Ok(all
            .into_iter()
            .filter(|e| e.text.to_lowercase().contains(&lower))
            .collect())
    }

    pub fn replace(&self, needle: &str, text: &str, session: &str) -> Result<Entry, StoreError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(StoreError::Invalid("text is empty".into()));
        }
        if text.contains('\n') || text.contains('\r') {
            return Err(StoreError::Invalid(
                "entry text must be one line — no newline".into(),
            ));
        }
        let matches = self.find_matches(needle)?;
        if matches.is_empty() {
            return Err(StoreError::NotFound(needle.into()));
        }
        if matches.len() > 1 {
            return Err(StoreError::Ambiguous(matches.len()));
        }
        let old = matches.into_iter().next().expect("one match");
        let tx = Tx::begin(&self.conn)?;
        let delta = text.chars().count() as isize - old.text.chars().count() as isize;
        let after = (self.usage(&old.track, &old.scope)? as isize + delta).max(0) as usize;
        if after > self.cap(&old.track) {
            return Err(StoreError::Budget {
                track: old.track.clone(),
                scope: old.scope.clone(),
                after,
                cap: self.cap(&old.track),
            });
        }
        self.conn.execute(
            "update entries set text=?1 where id=?2",
            rusqlite::params![text, old.id],
        )?;
        self.audit(
            "replace",
            &old.track,
            &old.scope,
            &old.id,
            text,
            "applied",
            "memory-tool",
            session,
        )?;
        tx.commit()?;
        self.get(&old.id)?
            .ok_or_else(|| StoreError::NotFound(old.id.clone()))
    }

    pub fn remove(&self, needle: &str, session: &str) -> Result<Entry, StoreError> {
        let matches = self.find_matches(needle)?;
        if matches.is_empty() {
            return Err(StoreError::NotFound(needle.into()));
        }
        if matches.len() > 1 {
            return Err(StoreError::Ambiguous(matches.len()));
        }
        let old = matches.into_iter().next().expect("one match");
        self.conn
            .execute("delete from entries where id=?1", rusqlite::params![old.id])?;
        self.audit(
            "remove",
            &old.track,
            &old.scope,
            &old.id,
            &old.text,
            "applied",
            "memory-tool",
            session,
        )?;
        Ok(old)
    }

    pub fn get(&self, id: &str) -> Result<Option<Entry>, StoreError> {
        let mut stmt = self.conn.prepare("select id,track,scope,workspace_key,agent_key,text,tags,source,created_at from entries where id=?1")?;
        let row = stmt
            .query_row(rusqlite::params![id], |r| {
                Ok(Entry {
                    id: r.get(0)?,
                    track: r.get(1)?,
                    scope: r.get(2)?,
                    workspace_key: r.get(3)?,
                    agent_key: r.get(4)?,
                    text: r.get(5)?,
                    tags: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                    source: r.get(7)?,
                    created_at: r.get(8)?,
                })
            })
            .optional()?;
        Ok(row)
    }

    /// Case-insensitive substring search over every entry, newest first.
    pub fn query(
        &self,
        pattern: &str,
        limit: usize,
        session: &str,
    ) -> Result<Vec<Entry>, StoreError> {
        let needle = pattern.to_lowercase();
        let mut hits: Vec<Entry> = self
            .list_entries()?
            .into_iter()
            .filter(|e| e.text.to_lowercase().contains(&needle))
            .collect();
        hits.sort_by_key(|e| std::cmp::Reverse(e.created_at));
        hits.truncate(limit.max(1));
        self.audit("recalled", "", "", "", "", "ok", "query", session)?;
        Ok(hits)
    }

    /// A consistent copy of the live database (VACUUM INTO replays the WAL), the merge's safety net.
    pub fn backup_to(&self, dest: &Path) -> Result<(), StoreError> {
        let dest = dest.display().to_string();
        self.conn
            .execute("VACUUM INTO ?1", rusqlite::params![dest])?;
        Ok(())
    }

    /// Consolidate several entries into one (the old maintenance script's `merge`): every match must name exactly one
    /// entry, the survivors are deleted, the replacement is inserted, and the audit records the whole move.
    pub fn merge(
        &self,
        matches: &[String],
        track: &str,
        scope: &str,
        text: &str,
        tags: &[String],
        source: &str,
    ) -> Result<String, StoreError> {
        if matches.is_empty() {
            return Err(StoreError::Invalid(
                "merge needs at least one --match".into(),
            ));
        }
        let text = text.trim();
        if text.is_empty() {
            return Err(StoreError::Invalid(
                "merge needs --text (the one entry that survives)".into(),
            ));
        }
        if text.contains('\n') || text.contains('\r') {
            return Err(StoreError::Invalid(
                "entry text must be one line — no newline".into(),
            ));
        }
        let all = self.list_entries()?;
        let mut victims = Vec::new();
        for m in matches {
            // Match only inside the destination track/scope, as the old maintenance merge did.
            let hits: Vec<&Entry> = all
                .iter()
                .filter(|e| e.track == track && e.scope == scope && e.text.contains(m.as_str()))
                .collect();
            match hits.len() {
                0 => return Err(StoreError::NotFound(m.clone())),
                1 => {
                    let hit = hits[0];
                    if victims.iter().any(|v: &&Entry| v.id == hit.id) {
                        return Err(StoreError::Invalid(format!(
                            "{m:?} resolves to the same entry as another match"
                        )));
                    }
                    victims.push(hit);
                }
                n => return Err(StoreError::Ambiguous(n)),
            }
        }
        // A consolidation replaces its victims: the budget is what the pair ends up holding, not
        // what it holds plus the whole new text. The check and the writes move in ONE transaction —
        // the usage is read after BEGIN IMMEDIATE, so a concurrent writer cannot slip past the cap
        // between the check and the deletes (v0.4.9; add and replace already worked this way).
        let victims_chars: usize = victims.iter().map(|v| v.text.chars().count()).sum();
        let id = uuid::Uuid::new_v4().to_string();
        let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".into());
        let tx = Tx::begin(&self.conn)?;
        let after = self.usage(track, scope)? - victims_chars + text.chars().count();
        let cap = self.cap(track);
        if after > cap {
            return Err(StoreError::Budget {
                track: track.into(),
                scope: scope.into(),
                after,
                cap,
            });
        }
        for v in &victims {
            self.conn
                .execute("delete from entries where id=?1", rusqlite::params![v.id])?;
            self.audit(
                "remove",
                &v.track,
                &v.scope,
                &v.id,
                &v.text,
                "consolidated",
                source,
                "",
            )?;
        }
        self.conn.execute(
            "insert into entries (id,track,scope,workspace_key,agent_key,text,tags,source,created_at) values (?1,?2,?3,'','',?4,?5,?6,?7)",
            rusqlite::params![id, track, scope, text, tags_json, source, Self::now_ms()],
        )?;
        self.audit(
            "consolidate",
            track,
            scope,
            &id,
            text,
            "applied",
            source,
            "",
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// One snapshot body per session: the caller freezes it.
    pub fn snapshot(&self, workspace_key: &str, agent_key: &str) -> Result<String, StoreError> {
        Ok(crate::snapshot::render_snapshot(
            &self.list_entries()?,
            &self.budgets,
            workspace_key,
            agent_key,
        ))
    }

    /// Import from a dsh-memento database; idempotent by id.
    pub fn import_from_memento(&self, memento_path: &Path) -> Result<usize, StoreError> {
        if !memento_path.exists() {
            return Ok(0);
        }
        // The source is read-only here too: importing must never write to the legacy store.
        let src =
            Connection::open_with_flags(memento_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut stmt = src.prepare("select id,track,scope,workspace_key,agent_key,text,tags,source,created_at from entries")?;
        let rows = stmt.query_map([], |r| {
            Ok(Entry {
                id: r.get(0)?,
                track: r.get(1)?,
                scope: r.get(2)?,
                workspace_key: r.get(3)?,
                agent_key: r.get(4)?,
                text: r.get(5)?,
                tags: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                source: r.get(7)?,
                created_at: r.get(8)?,
            })
        })?;
        let mut imported = 0;
        for row in rows {
            let e = row?;
            let exists: Option<String> = self
                .conn
                .query_row(
                    "select id from entries where id=?1",
                    rusqlite::params![e.id],
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_some() {
                continue;
            }
            // The caps hold at import too: a legacy entry that would blow the budget is skipped and
            // audited as such, never silently loaded past the store's own limits.
            let after = self.usage(&e.track, &e.scope)? + e.text.chars().count();
            if after > self.cap(&e.track) {
                self.audit(
                    "import",
                    &e.track,
                    &e.scope,
                    &e.id,
                    &e.text,
                    "skipped: over budget",
                    "migration",
                    "",
                )?;
                continue;
            }
            self.conn.execute(
                "insert into entries (id,track,scope,workspace_key,agent_key,text,tags,source,created_at) values (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                rusqlite::params![e.id, e.track, e.scope, e.workspace_key, e.agent_key, e.text, serde_json::to_string(&e.tags).unwrap_or_else(|_| "[]".into()), e.source, e.created_at],
            )?;
            self.audit(
                "import",
                &e.track,
                &e.scope,
                &e.id,
                &e.text,
                "imported from dsh-memento",
                "migration",
                "",
            )?;
            imported += 1;
        }
        Ok(imported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn merge_replaces_the_matches_inside_one_track_and_refuses_ambiguity() {
        let dir = std::env::temp_dir().join(format!("optimisemd-merge-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = Store::open(&dir.join("m.db"), Budgets::default()).unwrap();
        store
            .add(
                "user",
                "user-global",
                "Prefer short answers.",
                &[],
                "test",
                "",
                "",
                "",
            )
            .unwrap();
        store
            .add(
                "user",
                "user-global",
                "Prefer plain words.",
                &[],
                "test",
                "",
                "",
                "",
            )
            .unwrap();
        store
            .add(
                "agent",
                "workspace",
                "A fact that mentions Prefer.",
                &[],
                "test",
                "",
                "",
                "",
            )
            .unwrap();
        // the match only counts inside the destination track/scope, as the old merge did
        store
            .merge(
                &["Prefer short".into()],
                "user",
                "user-global",
                "Prefer short, plain answers.",
                &[],
                "test",
            )
            .unwrap();
        let texts: Vec<String> = store
            .list_entries()
            .unwrap()
            .into_iter()
            .map(|e| e.text)
            .collect();
        assert!(texts.contains(&"Prefer short, plain answers.".to_string()));
        assert!(!texts.contains(&"Prefer short answers.".to_string()));
        assert!(texts.contains(&"A fact that mentions Prefer.".to_string()));
        // "Prefer" now matches one user row and the agent row — but only the user row is in scope
        store
            .merge(
                &["Prefer".into()],
                "agent",
                "workspace",
                "A fact.",
                &[],
                "test",
            )
            .unwrap();
        let texts: Vec<String> = store
            .list_entries()
            .unwrap()
            .into_iter()
            .map(|e| e.text)
            .collect();
        assert!(texts.contains(&"A fact.".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod hardening_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn fresh(tag: &str) -> (PathBuf, Store) {
        let dir =
            std::env::temp_dir().join(format!("optimisemd-hard-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = Store::open(&dir.join("m.db"), Budgets::default()).unwrap();
        (dir, store)
    }

    #[test]
    fn merge_budget_counts_the_victims_it_replaces() {
        let (dir, store) = fresh("budget");
        let big = "A".repeat(1900);
        store
            .add("user", "user-global", &big, &[], "test", "", "", "")
            .unwrap();
        store
            .add(
                "user",
                "user-global",
                "B".repeat(50).as_str(),
                &[],
                "test",
                "",
                "",
                "",
            )
            .unwrap();
        // A consolidation that shrinks the pair must not be refused: 400 chars replace 1950.
        let id = store
            .merge(
                &["AAAA".into(), "BBBBB".into()],
                "user",
                "user-global",
                "C".repeat(400).as_str(),
                &[],
                "test",
            )
            .expect("a shrinking merge fits the cap");
        assert!(store.get(&id).unwrap().is_some());
        let (dir2, store2) = fresh("budget2");
        store2
            .add("user", "user-global", &big, &[], "test", "", "", "")
            .unwrap();
        // A merge that grows the pair past the cap is still refused.
        assert!(matches!(
            store2.merge(
                &["AAAA".into()],
                "user",
                "user-global",
                &"D".repeat(2500),
                &[],
                "test"
            ),
            Err(StoreError::Budget { .. })
        ));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn replace_refuses_empty_text() {
        let (dir, store) = fresh("empty");
        store
            .add(
                "user",
                "user-global",
                "Hello world",
                &[],
                "test",
                "",
                "",
                "",
            )
            .unwrap();
        assert!(matches!(
            store.replace("Hello", "   ", "s1"),
            Err(StoreError::Invalid(_))
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn entry_text_must_be_one_line() {
        let (dir, store) = fresh("newline");
        assert!(matches!(
            store.add("user", "user-global", "a\nb", &[], "test", "", "", ""),
            Err(StoreError::Invalid(_))
        ));
        store
            .add(
                "user",
                "user-global",
                "Hello world",
                &[],
                "test",
                "",
                "",
                "",
            )
            .unwrap();
        assert!(matches!(
            store.replace("Hello", "line one\nline two", "s1"),
            Err(StoreError::Invalid(_))
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_and_remove_match_case_insensitively() {
        let (dir, store) = fresh("case");
        store
            .add(
                "user",
                "user-global",
                "Hello World",
                &[],
                "test",
                "",
                "",
                "",
            )
            .unwrap();
        let e = store.replace("hello wor", "Howdy World", "s1").unwrap();
        assert_eq!(e.text, "Howdy World");
        store.remove("HOWDY", "s1").unwrap();
        assert!(store.list_entries().unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_skips_entries_over_the_cap() {
        let (dir, store) = fresh("import");
        let src_dir =
            std::env::temp_dir().join(format!("optimisemd-memento-{}", std::process::id()));
        let _ = fs::remove_dir_all(&src_dir);
        let src = Store::open(
            &src_dir.join("m.db"),
            Budgets {
                user: 100_000,
                agent: 100_000,
            },
        )
        .unwrap();
        src.add(
            "user",
            "user-global",
            &"X".repeat(5000),
            &[],
            "test",
            "",
            "",
            "",
        )
        .unwrap();
        src.add(
            "user",
            "user-global",
            "a small fact",
            &[],
            "test",
            "",
            "",
            "",
        )
        .unwrap();
        let n = store.import_from_memento(&src_dir.join("m.db")).unwrap();
        assert_eq!(n, 1, "the over-cap entry must not import");
        assert_eq!(
            store.usage("user", "user-global").unwrap(),
            "a small fact".len()
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&src_dir);
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use std::fs;

    #[test]
    fn history_lists_every_operation_newest_first() {
        let dir = std::env::temp_dir().join(format!("optimisemd-history-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = Store::open(&dir.join("m.db"), Budgets::default()).unwrap();
        let tags: Vec<String> = Vec::new();
        store
            .add(
                "user",
                "user-global",
                "first fact",
                &tags,
                "test",
                "/ws",
                "",
                "s1",
            )
            .unwrap();
        store.replace("first", "second fact", "s1").unwrap();
        store.remove("second", "s1").unwrap();
        store.query("second", 5, "s1").unwrap();
        let rows = store.history(10).unwrap();
        let actions: Vec<&str> = rows.iter().map(|r| r.action.as_str()).collect();
        assert_eq!(actions, ["recalled", "remove", "replace", "add"]);
    }
}
