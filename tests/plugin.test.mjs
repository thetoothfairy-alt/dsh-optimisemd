// The JS surface: the addon is found, and the tool's execute honours the store.
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { loadNative, nativeFile } from '../lib/native.mjs'
import { executeMemoryAction } from '../lib/action.mjs'
import { executeRepoAction, localDate } from '../lib/repo-action.mjs'
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs'

test('nativeFile names one artifact per platform', () => {
  assert.equal(nativeFile('linux', 'x64'), 'dsh-optimisemd.linux-x64.node')
  assert.equal(nativeFile('darwin', 'arm64'), 'dsh-optimisemd.darwin-arm64.node')
  assert.equal(nativeFile('win32', 'x64'), 'dsh-optimisemd.win32-x64.node')
})

test('a missing addon says which file and how to build it', () => {
  assert.throws(() => loadNative({ base: '/nonexistent' }), /native addon not found/)
})

test('the tool executes against the store and refuses empty text', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'dsh-optimisemd-tool-'))
  try {
    const { MemoryStore } = loadNative()
    const store = new MemoryStore(join(dir, 'memory.db'), 2000, 4000)
    const exec = { signal: { throwIfAborted() {} }, agent: { session: { id: 's1', header: { cwd: dir, agentPreset: '' } } } }
    const added = await executeMemoryAction(store, { action: 'add', track: 'user', scope: 'user-global', text: 'plain words', tags: ['pref'] }, exec)
    assert.equal(added.ok, true)
    const bad = await executeMemoryAction(store, { action: 'add', track: 'user', scope: 'user-global', text: '   ' }, exec)
    assert.equal(bad.ok, false)
    assert.match(bad.error, /empty/)
    const found = await executeMemoryAction(store, { action: 'query', text: 'plain' }, exec)
    assert.equal(found.entries.length, 1)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('memory_save writes the row, the part section and the day line', async () => {
  const { RepoWriter } = loadNative()
  const dir = mkdtempSync(join(tmpdir(), 'optimisemd-save-'))
  try {
    mkdirSync(join(dir, 'docs/memory-details'), { recursive: true })
    writeFileSync(join(dir, 'MEMORY.md'), '# MEMORY.md\n\n## Subject facts\n- **An old row.** (2026-01-01) — `docs/memory-details/01.md`\n')
    const exec = { signal: { throwIfAborted() {} }, agent: { session: { id: 's1', header: { cwd: dir, agentPreset: '' } } } }
    const out = await executeRepoAction(RepoWriter, { text: 'A saved fact.', pointer: 'docs/memory-details/02.md', part: '02.md', slug: 'saved-fact', detail: 'Long form.' }, exec)
    assert.equal(out.ok, true, out.error)
    assert.match(readFileSync(join(dir, 'MEMORY.md'), 'utf8'), /A saved fact\.\*\* \(\d{4}-\d{2}-\d{2}\)/)
    assert.match(readFileSync(join(dir, 'docs/memory-details/02.md'), 'utf8'), /### §saved-fact/)
    assert.ok(readFileSync(join(dir, 'memory', `${localDate()}.md`), 'utf8').includes('A saved fact.'))
    const bad = await executeRepoAction(RepoWriter, { text: 'x', pointer: 'notes/elsewhere.md' }, exec)
    assert.equal(bad.ok, false)
    assert.match(bad.error, /pointer refused/)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
