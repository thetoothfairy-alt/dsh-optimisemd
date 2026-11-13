//! optimisemd-core — the tier-0 store and snapshot renderer for `dsh-optimisemd`.
//!
//! One writer: this crate owns `$DSH_HOME/dsh-optimisemd/memory.db`. The plugin around it is a thin
//! Node surface; every rule (caps, unique-match, audit) lives here so the harness and the CLI
//! cannot disagree about what the store means.

pub mod promo;
pub mod repo;
pub mod snapshot;
pub mod store;

pub use promo::{Promoter, Report, Row};
pub use repo::{Repo, RepoError, Written};
pub use snapshot::render_snapshot;
pub use store::{Budgets, Entry, Store, StoreError};
