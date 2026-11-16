// The JS surface: the addon is found, and the tool's execute honours the store.
import { mkdtempSync, rmSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
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
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
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
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
  }
})

test('the query limit is validated before it reaches the store', async () => {
  const { MemoryStore } = loadNative()
  const dir = mkdtempSync(join(tmpdir(), 'dsh-optimisemd-limit-'))
  try {
    const store = new MemoryStore(join(dir, 'memory.db'), 2000, 4000)
    const exec = { signal: { throwIfAborted() {} }, agent: { session: { id: 's1', header: { cwd: dir, agentPreset: '' } } } }
    for (const limit of ['abc', -5, Infinity, NaN, 2.5]) {
      const out = await executeMemoryAction(store, { action: 'query', text: 'x', limit }, exec)
      assert.equal(out.ok, false, `limit ${limit} must be refused`)
      assert.match(out.error, /limit/)
    }
  } finally {
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
  }
})

test('memory_save refuses a part that would escape the parts directory', async () => {
  const { RepoWriter } = loadNative()
  const dir = mkdtempSync(join(tmpdir(), 'optimisemd-escape-'))
  try {
    mkdirSync(join(dir, 'docs/memory-details'), { recursive: true })
    writeFileSync(join(dir, 'MEMORY.md'), '# MEMORY.md\n\n## Subject facts\n')
    const exec = { signal: { throwIfAborted() {} }, agent: { session: { id: 's1', header: { cwd: dir, agentPreset: '' } } } }
    const out = await executeRepoAction(RepoWriter, { text: 'A fact.', pointer: 'docs/memory-details/02.md', part: '../evil.md', slug: 'x', detail: 'd' }, exec)
    assert.equal(out.ok, false)
    assert.match(out.error, /part/)
    assert.ok(!existsSync(join(dir, 'docs/evil.md')))
  } finally {
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
  }
})

test('the snapshot section is literal and frozen for the session', async () => {
  const { apply } = await import('../index.mjs')
  const { MemoryStore } = loadNative()
  const dir = mkdtempSync(join(tmpdir(), 'optimisemd-freeze-'))
  try {
    let captured
    const ctx = {
      tools: { register() {} },
      systemPrompt: { section(def) { captured = def } },
    }
    apply(ctx, { dbPath: join(dir, 'memory.db') })
    assert.equal(captured.interpolate, false, 'the section must not be interpolated')
    const assemble = { agent: { session: { id: 's1', header: { cwd: dir, agentPreset: '' } } } }
    const first = captured.text(assemble)
    assert.match(first, /Tier-0 memory/)
    const store = new MemoryStore(join(dir, 'memory.db'), 2000, 4000)
    store.add('agent', 'workspace', 'a mid-session write', [], 'test', resolve(dir), '', 's1')
    const second = captured.text(assemble)
    assert.equal(second, first, 'a mid-session write must not change the frozen snapshot')
    // A new session sees the write.
    const assemble2 = { agent: { session: { id: 's2', header: { cwd: dir, agentPreset: '' } } } }
    assert.match(captured.text(assemble2), /a mid-session write/)
  } finally {
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
  }
})
