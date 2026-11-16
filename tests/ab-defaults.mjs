// A/B the plugin tools' defaults: the installed 0.4.5 package decides track/scope and the save date in JS; this repo
// decides them in Rust. Both run the same handlers on identical reset fixtures, and the results, store and files are
// compared. Run: node tests/ab-defaults.mjs   (AB_A overrides the installed package path.)
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const HERE = dirname(dirname(fileURLToPath(import.meta.url)))
const A = process.env.AB_A || '/home/admin/.dsh/profiles/web/node_modules/dsh-optimisemd'
const B = HERE
const BIN = '/home/admin/.openclaw/workspace/bin/dsh-optimisemd'
const BASE = '/tmp/ab-dsh-optimisemd'

const normalize = (value, base) => {
  const s = JSON.stringify(value)
    .split(base).join('<base>')
  const parsed = JSON.parse(s)
  const walk = (v) => {
    if (Array.isArray(v)) return v.map(walk)
    if (v && typeof v === 'object') {
      const out = {}
      for (const [k, x] of Object.entries(v)) out[k] = k === 'id' ? '<id>' : k === 'createdAt' ? '<time>' : walk(x)
      return out
    }
    return v
  }
  return walk(parsed)
}

const fixture = (root) => {
  mkdirSync(join(root, 'docs/memory-details'), { recursive: true })
  mkdirSync(join(root, 'scripts'), { recursive: true })
  writeFileSync(
    join(root, 'MEMORY.md'),
    '# MEMORY.md\n\n## Subject facts\n- **An existing row.** (2026-01-01) — `docs/memory-details/01.md`\n',
  )
  writeFileSync(join(root, 'scripts/check-memory-index.sh'), '#!/usr/bin/env bash\nexit 0\n')
}

const runSide = async (side, root) => {
  const base = join(BASE, side)
  rmSync(base, { recursive: true, force: true })
  const dshhome = join(base, 'dshhome')
  const workspace = join(base, 'workspace')
  const storePath = join(dshhome, 'dsh-optimisemd', 'memory.db')
  mkdirSync(join(storePath, '..'), { recursive: true })
  mkdirSync(workspace, { recursive: true })
  fixture(workspace)

  const native = await import(pathToFileURL(join(root, 'lib/native.mjs')).href)
  const action = await import(pathToFileURL(join(root, 'lib/action.mjs')).href)
  const repo = await import(pathToFileURL(join(root, 'lib/repo-action.mjs')).href)
  const addon = native.loadNative()
  const store = new addon.MemoryStore(storePath, 2000, 4000)
  const exec = { agent: { session: { id: 'ab-session', header: { cwd: workspace, agentPreset: 'web' } } } }
  const log = (name, value) => ({ name, value: normalize(value, base) })

  const out = []
  const add = await action.executeMemoryAction(store, { action: 'add', text: 'A fact for the A/B.' }, exec)
  out.push(log('add', add))
  out.push(log('entries-after-add', { entries: store.listEntries(), usage: store.usage() }))
  const binary = execFileSync(BIN, ['tier0', 'list'], {
    env: { ...process.env, DSH_HOME: dshhome },
    encoding: 'utf8',
  })
  out.push({ name: 'binary-read', value: binary.replace(storePath, '<store>') })

  out.push(log('query', await action.executeMemoryAction(store, { action: 'query', text: 'A/B' }, exec)))
  out.push(log('replace', await action.executeMemoryAction(store, { action: 'replace', match: 'A fact for the A/B', text: 'A replaced fact.' }, exec)))
  out.push(log('remove', await action.executeMemoryAction(store, { action: 'remove', match: 'A replaced fact' }, exec)))
  out.push(log('entries-after-remove', { entries: store.listEntries(), usage: store.usage() }))

  const saved = await repo.executeRepoAction(addon.RepoWriter, { text: 'A saved fact for the A/B.', pointer: 'docs/memory-details/01.md' }, exec)
  out.push(log('save', saved))
  const files = {
    memory: readFileSync(join(workspace, 'MEMORY.md'), 'utf8'),
    day: saved.day ? readFileSync(saved.day, 'utf8') : null,
  }
  const auditLog = readFileSync(join(workspace, 'memory/logs/repo-writes.log'), 'utf8')
    .split('\n')
    .filter(Boolean)
    .map((l) => l.replace(/^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d /, '<time> '))
  out.push({ name: 'files', value: { memory: files.memory, day: files.day } })
  out.push({ name: 'repo-writes-log', value: auditLog })

  out.push(log('refused-save', await repo.executeRepoAction(addon.RepoWriter, { text: 'A refused fact.', pointer: 'not-a-pointer' }, exec)))
  return out
}

const a = await runSide('A', A)
const b = await runSide('B', B)
const keys = a.map((x) => x.name)
const report = keys.map((name, i) => ({
  scenario: name,
  equal: JSON.stringify(a[i].value) === JSON.stringify(b[i].value),
  a: a[i].value,
  b: b[i].value,
}))
const allEqual = report.every((r) => r.equal)
console.log(JSON.stringify({ a: A, b: B, allEqual, scenarios: report }, null, 2))
writeFileSync(join(BASE, 'report.json'), JSON.stringify({ allEqual, scenarios: report }, null, 2))
if (!allEqual) process.exitCode = 1
