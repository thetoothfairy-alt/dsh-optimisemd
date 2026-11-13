// The addon is the whole mechanism: caps, unique-match, audit, snapshot visibility.
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { loadNative } from '../lib/native.mjs'

const { MemoryStore } = loadNative()
const fresh = () => {
  const dir = mkdtempSync(join(tmpdir(), 'dsh-optimisemd-'))
  return { store: new MemoryStore(join(dir, 'memory.db'), 2000, 4000), dir }
}

test('add, usage, snapshot and query round-trip', () => {
  const { store, dir } = fresh()
  try {
    store.add('user', 'user-global', 'Prefer small, plain answers.', ['pref'], 'test', '/ws', '', 's1')
    assert.deepEqual(store.usage(), { userUsed: 28, userCap: 2000, agentUsed: 0, agentCap: 4000 })
    const snap = store.snapshot('/ws', '')
    assert.match(snap, /Tier-0 memory — user\/user-global 28\/2000/)
    assert.match(snap, /\[user\/user-global\]\n- Prefer small, plain answers\./)
    assert.deepEqual(store.query('plain', 5, 's1').map((e) => e.text), ['Prefer small, plain answers.'])
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('the cap refuses instead of growing; consolidate is the only route', () => {
  const { store, dir } = fresh()
  try {
    store.add('user', 'user-global', 'x'.repeat(1990), [], 'test', '/ws', '', 's1')
    assert.throws(() => store.add('user', 'user-global', 'y'.repeat(20), [], 'test', '/ws', '', 's1'), /budget/)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('replace and remove need exactly one match', () => {
  const { store, dir } = fresh()
  try {
    store.add('agent', 'workspace', 'first fact about the finder', [], 'test', '/ws', '', 's1')
    store.add('agent', 'workspace', 'second fact about the finder', [], 'test', '/ws', '', 's1')
    assert.throws(() => store.replace('finder', 'new text', 's1'), /ambiguous/)
    const replaced = store.replace('first fact', 'first fact, corrected', 's1')
    assert.equal(replaced.text, 'first fact, corrected')
    assert.throws(() => store.remove('nothing here', 's1'), /not found/)
    const removed = store.remove('second fact', 's1')
    assert.match(removed.text, /second fact/)
    assert.equal(store.listEntries().length, 1)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('a workspace entry is invisible in another workspace', () => {
  const { store, dir } = fresh()
  try {
    store.add('user', 'user-global', 'everywhere', [], 'test', '/ws', '', 's1')
    store.add('agent', 'workspace', 'only here', [], 'test', '/ws', '', 's1')
    const other = store.snapshot('/elsewhere', '')
    assert.match(other, /everywhere/)
    assert.doesNotMatch(other, /only here/)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
