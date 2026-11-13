//! Native addon surface for the `dsh-optimisemd` plugin. Everything crosses as plain data; no async,
//! no threads: the store is a local SQLite file and the calls are microseconds.

use napi_derive::napi;
use optimisemd_core::{Budgets, Entry, Repo, Store};
use std::path::Path;

#[napi(object)]
pub struct NativeEntry {
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

impl From<Entry> for NativeEntry {
    fn from(e: Entry) -> Self {
        Self {
            id: e.id,
            track: e.track,
            scope: e.scope,
            workspace_key: e.workspace_key,
            agent_key: e.agent_key,
            text: e.text,
            tags: e.tags,
            source: e.source,
            created_at: e.created_at,
        }
    }
}

#[napi(object)]
pub struct Usage {
    pub user_used: u32,
    pub user_cap: u32,
    pub agent_used: u32,
    pub agent_cap: u32,
}

fn fail(e: optimisemd_core::StoreError) -> napi::Error {
    napi::Error::from_reason(e.to_string())
}

#[napi]
pub struct MemoryStore {
    inner: Store,
}

#[napi]
impl MemoryStore {
    #[napi(constructor)]
    pub fn new(path: String, user_cap: Option<u32>, agent_cap: Option<u32>) -> napi::Result<Self> {
        let budgets = Budgets {
            user: user_cap.map(|v| v as usize).unwrap_or(2000),
            agent: agent_cap.map(|v| v as usize).unwrap_or(4000),
        };
        let inner = Store::open(Path::new(&path), budgets).map_err(fail)?;
        Ok(Self { inner })
    }

    #[napi]
    pub fn list_entries(&self) -> napi::Result<Vec<NativeEntry>> {
        Ok(self
            .inner
            .list_entries()
            .map_err(fail)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    // The addon mirrors the store's fields one-to-one; napi needs them as separate arguments.
    #[allow(clippy::too_many_arguments)]
    #[napi]
    pub fn add(
        &self,
        track: String,
        scope: String,
        text: String,
        tags: Vec<String>,
        source: String,
        workspace_key: String,
        agent_key: String,
        session: String,
    ) -> napi::Result<String> {
        self.inner
            .add(
                &track,
                &scope,
                &text,
                &tags,
                &source,
                &workspace_key,
                &agent_key,
                &session,
            )
            .map_err(fail)
    }

    #[napi]
    pub fn replace(
        &self,
        needle: String,
        text: String,
        session: String,
    ) -> napi::Result<NativeEntry> {
        self.inner
            .replace(&needle, &text, &session)
            .map(Into::into)
            .map_err(fail)
    }

    #[napi]
    pub fn remove(&self, needle: String, session: String) -> napi::Result<NativeEntry> {
        self.inner
            .remove(&needle, &session)
            .map(Into::into)
            .map_err(fail)
    }

    #[napi]
    pub fn query(
        &self,
        pattern: String,
        limit: u32,
        session: String,
    ) -> napi::Result<Vec<NativeEntry>> {
        Ok(self
            .inner
            .query(&pattern, limit as usize, &session)
            .map_err(fail)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    #[napi]
    pub fn usage(&self) -> napi::Result<Usage> {
        Ok(Usage {
            user_used: self.inner.usage("user", "user-global").map_err(fail)? as u32,
            user_cap: self.inner.cap("user") as u32,
            agent_used: self.inner.usage("agent", "workspace").map_err(fail)? as u32,
            agent_cap: self.inner.cap("agent") as u32,
        })
    }

    #[napi]
    pub fn snapshot(&self, workspace_key: String, agent_key: String) -> napi::Result<String> {
        self.inner
            .snapshot(&workspace_key, &agent_key)
            .map_err(fail)
    }

    #[napi]
    pub fn import_from_memento(&self, path: String) -> napi::Result<u32> {
        Ok(self
            .inner
            .import_from_memento(Path::new(&path))
            .map_err(fail)? as u32)
    }
}

#[napi(object)]
pub struct Written {
    pub row: String,
    pub part: Option<String>,
    pub day: Option<String>,
}

/// The repo stores: MEMORY.md rows, detail-part sections and the day's line. This is P2 — one call creates a fact in the
/// repository instead of an edit plus three checks.
#[napi]
pub struct RepoWriter {
    inner: Repo,
}

#[napi]
impl RepoWriter {
    #[napi(constructor)]
    pub fn new(root: String) -> Self {
        Self { inner: Repo::new(root) }
    }

    // The tool exposes one create call; the store's fields are its arguments.
    #[allow(clippy::too_many_arguments)]
    #[napi]
    pub fn save(
        &self,
        text: String,
        date: String,
        pointer: String,
        section: Option<String>,
        part: Option<String>,
        slug: Option<String>,
        detail: Option<String>,
    ) -> napi::Result<Written> {
        let w = self
            .inner
            .save(
                &text,
                &date,
                &pointer,
                section.as_deref(),
                part.as_deref(),
                slug.as_deref(),
                detail.as_deref(),
            )
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(Written { row: w.row, part: w.part, day: w.day })
    }

    #[napi]
    pub fn append_day(&self, date: String, text: String) -> napi::Result<String> {
        self.inner
            .append_day(&date, &text)
            .map(|p| p.display().to_string())
            .map_err(|e| napi::Error::from_reason(e.to_string()))
    }
}
