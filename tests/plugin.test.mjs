// The JS surface: the addon is found, and the tool's execute honours the store.
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { loadNative, nativeFile } from '../lib/native.mjs'
import { executeMemoryAction } from '../lib/action.mjs'

test('nativeFile names one artifact per platform', () => {
  assert.equal(nativeFile('linux', 'x64'), 'dsh-memory.linux-x64.node')
  assert.equal(nativeFile('darwin', 'arm64'), 'dsh-memory.darwin-arm64.node')
  assert.equal(nativeFile('win32', 'x64'), 'dsh-memory.win32-x64.node')
})

test('a missing addon says which file and how to build it', () => {
  assert.throws(() => loadNative({ base: '/nonexistent' }), /native addon not found/)
})

test('the tool executes against the store and refuses empty text', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'dsh-memory-tool-'))
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
