//! memory-core — the tier-0 store and snapshot renderer for `dsh-memory`.
//!
//! One writer: this crate owns `$DSH_HOME/dsh-memory/memory.db`. The plugin around it is a thin
//! Node surface; every rule (caps, unique-match, audit) lives here so the harness and the CLI
//! cannot disagree about what the store means.

pub mod snapshot;
pub mod store;

pub use snapshot::render_snapshot;
pub use store::{Budgets, Entry, Store, StoreError};
