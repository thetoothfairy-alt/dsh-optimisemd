# dsh-optimisemd

Tier-0 memory for DeepSeek Harness: a small, capped store that every session reads once at its first
assembly, plus a `memory` tool to change it. The store and its rules are Rust; the plugin is a thin
host surface.

This is a redesign of a script-based memory set (promotion, demotion, creation, checks) into one
plugin package. The package is versioned in GitHub; CI tests it and releases prebuilt addons per
platform.

## Tools

| Tool | Writes | When |
|---|---|---|
| `memory` | tier 0, the capped store (`add`/`replace`/`remove`/`query`) | preferences and the workspace pointer |
| `memory_save` | **the repository** — a dated `MEMORY.md` row (≤200 chars), optionally its `docs/memory-details` section, and the `memory/<date>.md` line | facts; refuses duplicates, over-long rows and pointers that are not a part/runbook/skill/landmine index |

## Command line

The same crate ships `dsh-optimisemd`, the binary the workspace's nightlies call:

```
dsh-optimisemd promote            # demote + promote candidates (report)
dsh-optimisemd promote --apply    # move quiet rows into their part, with the check as the gate
dsh-optimisemd tier0 list|add|replace|remove|query|verify|history
dsh-optimisemd check              # the workspace's index check
dsh-optimisemd export             # memory/logs/memento-export.md from a read-only store copy
dsh-optimisemd index              # refresh the ranked finder
dsh-optimisemd slim               # markdown slimming report (budgets parsed from the workspace guard)
dsh-optimisemd counts             # check the measured block in docs/memory-process.md
dsh-optimisemd counts --write     # regenerate it (the workspace's nightly does this and commits the line)
dsh-optimisemd session-index      # docs/session-index.md; --check fails when stale
dsh-optimisemd session-commits    # docs/session-commits.md over --days
dsh-optimisemd session-digest     # memory/logs/session-digest.md
dsh-optimisemd log-index          # docs/memory-log-index.md
dsh-optimisemd landmine add …     # one hard-won line + its reproduction, guard as the gate
dsh-optimisemd check [paths…]     # the MEMORY.md index rules (the bash gate wraps this)
```

## Architecture

```
session first assembly ──▶ systemPrompt section (order -50) ──▶ frozen snapshot (Rust render)
model calls memory tool ──▶ lib/action.mjs ──▶ native addon ──▶ rusqlite (WAL) + audit table
```

- `crates/optimisemd-core` — the store: caps (user 2000 / agent 4000 chars), unique-substring
  replace/remove, audit rows, snapshot rendering, import from `dsh-memento`.
- `crates/optimisemd-node` — the N-API addon (napi-rs), exposed as `MemoryStore`.
- `lib/native.mjs`, `lib/action.mjs` — host-free halves: addon lookup, DB path, tool body.
- `index.mjs` — the cordis plugin: `apply` registers the tool and the snapshot section.
- `cordis.patch.yml` — the bundle patch `dsh` composes.

The snapshot is frozen per session (a WeakMap keyed by the session), so a mid-session write never
changes the prompt prefix.

## Install

From a tag (the release attaches the linux-x64 and win32-x64 addons; the darwin-arm64 addon is built on a Mac and attached
to the same release — `scripts/build-macos-air.sh` needs `AIR_HOST` set to its ssh alias, which is deliberately not in this
repository). `prepare` keeps the release's prebuilt addon when it matches the platform and only builds from source when
it is missing (or `pnpm build` forces a rebuild):

```sh
cd ~/.dsh/profiles/web
dsh plugin --profile web add github:thetoothfairy-alt/dsh-optimisemd#v0.4.3
```

`dsh` prints the exact `allowBuilds` key to paste into the profile's `pnpm-workspace.yaml` the first time (pnpm blocks
git-hosted build scripts until allowed); paste it and re-run the command. `pnpm peers check` then reports `@deepseek-ai/cordis`
and `@deepseek-ai/dsh-tools` as missing peers — the harness supplies them at runtime, the same way `dsh-memento` gets them.

From a checkout (development — needs the Rust toolchain; `prepare` builds the addon):

```sh
dsh plugin --profile web add link:/path/to/dsh-optimisemd
bash scripts/build-native.sh
```

`dsh plugin add` also appends the package to the profile's `dsh.profile.bundles`, and its own
`cordis.patch.yml` mounts the row; verify with `dsh --profile <name> --dump-config | grep -A3 'id: memory'`.
Per-profile overrides (for example a different database file) go in the profile's `cordis.patch.yml`:

```yaml
- id: memory
  config:
    dbPath: /tmp/scratch-memory.db
```

## Gated saves need a POSIX shell

`memory_save` runs the workspace's `scripts/check-memory-index.sh` before a save stands, and `find`
runs the workspace's finder. Both are shell scripts. Linux and macOS always have `/bin/sh`; **on
Windows a POSIX shell means Git for Windows** (its `bash` is looked for at the standard install
locations, so it does not need to be on `PATH`). Without any shell the gate **cannot run, so the
save is refused and rolled back** — that is the safe behaviour, not a failure. The Windows test
matrix runs the shell-backed tests with `REQUIRE_SHELL=1`, so a runner that loses its shell is a
red run rather than a silent skip.

## Build and test

```sh
pnpm build     # cargo build --release + native/dsh-optimisemd.<os>-<arch>.node
pnpm test      # node --test tests/*.test.mjs
cargo test     # store + snapshot unit tests
```

## Releasing

A release tag is written after the gates are green, and once a release exists it is never moved or
force-pushed: the fix is a new version, because a tag is what people pin and what caches hold. A tag
that has never had a release attached may be re-pointed when CI finds a fault (0.4.9 was, twice),
and the release record says which commit it finally named.

- CI (`release.yml`) runs format, clippy, the Rust tests and the host tests on Ubuntu **and**
  Windows, then builds the linux-x64 and win32-x64 install files from the tag.
- The macOS install file is built on a Mac from the tag (`scripts/build-macos-air.sh <tag>`) and
  uploaded to the same release by hand — there is no macOS runner on the plan, so that artifact is
  the one part without CI provenance. This is unchanged since 0.4.5.
- Publish is blocked by a red test run and by a tag/version mismatch (`package.json` and
  `Cargo.toml` must equal the tag's version).

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
