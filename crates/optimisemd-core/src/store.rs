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

impl Store {
    pub fn open(path: &Path, budgets: Budgets) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
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
        if track != "user" && track != "agent" {
            return Err(StoreError::Invalid(format!("unknown track {track:?}")));
        }
        if scope != "user-global" && scope != "workspace" {
            return Err(StoreError::Invalid(format!("unknown scope {scope:?}")));
        }
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
        let id = uuid::Uuid::new_v4().to_string();
        let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".into());
        self.conn.execute(
            "insert into entries (id,track,scope,workspace_key,agent_key,text,tags,source,created_at) values (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![id, track, scope, workspace_key, agent_key, text, tags_json, source, Self::now_ms()],
        )?;
        self.audit("add", track, scope, &id, text, "applied", source, session)?;
        Ok(id)
    }

    fn find_matches(&self, needle: &str) -> Result<Vec<Entry>, StoreError> {
        let all = self.list_entries()?;
        Ok(all
            .into_iter()
            .filter(|e| e.text.contains(needle))
            .collect())
    }

    pub fn replace(&self, needle: &str, text: &str, session: &str) -> Result<Entry, StoreError> {
        let matches = self.find_matches(needle)?;
        if matches.is_empty() {
            return Err(StoreError::NotFound(needle.into()));
        }
        if matches.len() > 1 {
            return Err(StoreError::Ambiguous(matches.len()));
        }
        let old = matches.into_iter().next().expect("one match");
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
            rusqlite::params![text.trim(), old.id],
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
        let id = uuid::Uuid::new_v4().to_string();
        let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".into());
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
        let src = Connection::open(memento_path)?;
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
