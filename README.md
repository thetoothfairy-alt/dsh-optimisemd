# dsh-memory

Tier-0 memory for DeepSeek Harness: a small, capped store that every session reads once at its first
assembly, plus a `memory` tool to change it. The store and its rules are Rust; the plugin is a thin
host surface.

This is a redesign of a script-based memory set (promotion, demotion, creation, checks) into one
plugin package. The package is versioned in GitHub; CI tests it and releases prebuilt addons per
platform.

## Architecture

```
session first assembly ──▶ systemPrompt section (order -50) ──▶ frozen snapshot (Rust render)
model calls memory tool ──▶ lib/action.mjs ──▶ native addon ──▶ rusqlite (WAL) + audit table
```

- `crates/memory-core` — the store: caps (user 2000 / agent 4000 chars), unique-substring
  replace/remove, audit rows, snapshot rendering, import from `dsh-memento`.
- `crates/memory-node` — the N-API addon (napi-rs), exposed as `MemoryStore`.
- `lib/native.mjs`, `lib/action.mjs` — host-free halves: addon lookup, DB path, tool body.
- `index.mjs` — the cordis plugin: `apply` registers the tool and the snapshot section.
- `cordis.patch.yml` — the bundle patch `dsh` composes.

The snapshot is frozen per session (a WeakMap keyed by the session), so a mid-session write never
changes the prompt prefix.

## Install

From a tag (the release attaches one `.node` per target; nothing is built at install time):

```sh
cd ~/.dsh/profiles/web
dsh plugin --profile web add github:bwra/dsh-memory#v0.1.0
```

From a checkout (development — needs the Rust toolchain; `prepare` builds the addon):

```sh
dsh plugin --profile web add link:/path/to/dsh-memory
bash scripts/build-native.sh
```

Mount it in the profile's `cordis.patch.yml` (a bundle deliberately does not auto-mount):

```yaml
- insert:
    - id: memory
      name: dsh-memory
      config:
        snapshotOrder: -50
```

## Build and test

```sh
pnpm build     # cargo build --release + native/dsh-memory.<os>-<arch>.node
pnpm test      # node --test tests/*.test.mjs
cargo test     # store + snapshot unit tests
```

## Migrating from dsh-memento

`MemoryStore.importFromMemento(path)` copies entries idempotently (same ids, audit row per import).
The migration procedure — copy the store, switch the profile, keep the old package installed for one
week, roll back by removing the row — is in the design note this repo was created from
(`docs/plans/2026-11-13-memory-plugin.md` in the workspace that owns it).

## Roadmap (the rest of the redesign)

1. **Repo stores** — `MEMORY.md` rows, `docs/memory-details/` parts and the daily log as first-class
   objects, so creation is one call instead of an edit plus three checks.
2. **Promotion and demotion** — the policy and its guards ported from the Python scripts, with a
   dry-run that prints the exact diff.
3. **Checks as a gate** — a `memory check` command that runs the existing checkers before any write
   lands, so automation cannot commit an invalid index.
4. **Optimisation** — export and index maintenance from the same binary (the finder's index becomes
   a subscriber, not a separate cron step).
