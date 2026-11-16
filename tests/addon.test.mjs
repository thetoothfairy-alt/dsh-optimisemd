// The addon is the whole mechanism: caps, unique-match, audit, snapshot visibility.
import { chmodSync, existsSync, mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { loadNative } from '../lib/native.mjs'

const { MemoryStore, RepoWriter } = loadNative()
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
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
  }
})

test('the Rust addon supplies the tool defaults', () => {
  const dir = mkdtempSync(join(tmpdir(), 'dsh-optimisemd-default-'))
  try {
    const store = new MemoryStore(join(dir, 'memory.db'), 2000, 4000)
    store.add('', '', 'A defaulted fact.', [], 'test', '/ws', '', 's1')
    const hit = store.query('defaulted', 5, 's1')[0]
    assert.equal(hit.track, 'agent')
    assert.equal(hit.scope, 'workspace')

    mkdirSync(join(dir, 'docs/memory-details'), { recursive: true })
    mkdirSync(join(dir, 'scripts'), { recursive: true })
    writeFileSync(
      join(dir, 'MEMORY.md'),
      '# MEMORY.md\n\n## Subject facts\n- **An existing row.** (2026-01-01) — `docs/memory-details/01.md`\n',
    )
    writeFileSync(join(dir, 'scripts/check-memory-index.sh'), '#!/usr/bin/env bash\nexit 0\n')
    const written = new RepoWriter(dir).saveChecked(
      'A saved fact.',
      '',
      'docs/memory-details/01.md',
      null,
      null,
      null,
      null,
    )
    const today = new Date()
    const iso = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, '0')}-${String(today.getDate()).padStart(2, '0')}`
    assert.equal(written.day, join(dir, 'memory', `${iso}.md`))
    assert.ok(readFileSync(written.day, 'utf8').includes('A saved fact.'))
  } finally {
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
  }
})

test('a bad date command is refused before any write', () => {
  const dir = mkdtempSync(join(tmpdir(), 'dsh-optimisemd-baddate-'))
  const bin = join(dir, 'bin')
  try {
    mkdirSync(bin, { recursive: true })
    mkdirSync(join(dir, 'docs/memory-details'), { recursive: true })
    mkdirSync(join(dir, 'scripts'), { recursive: true })
    writeFileSync(
      join(dir, 'MEMORY.md'),
      '# MEMORY.md\n\n## Subject facts\n- **An existing row.** (2026-01-01) — `docs/memory-details/01.md`\n',
    )
    writeFileSync(join(dir, 'scripts/check-memory-index.sh'), '#!/usr/bin/env bash\nexit 0\n')
    const before = readFileSync(join(dir, 'MEMORY.md'), 'utf8')
    const oldPath = process.env.PATH
    const stub = (body) => {
      writeFileSync(join(bin, 'date'), `#!/usr/bin/env bash\n${body}\n`)
      chmodSync(join(bin, 'date'), 0o755)
    }
    const attempt = (dateArg) => {
      try {
        new RepoWriter(dir).saveChecked('A fact.', dateArg, 'docs/memory-details/01.md', null, null, null, null)
        return null
      } catch (e) {
        return String(e?.message ?? e)
      }
    }
    try {
      process.env.PATH = `${bin}:${oldPath}`
      stub('echo 2026-11-16; exit 1')
      assert.match(attempt(''), /date command failed/)
      assert.equal(readFileSync(join(dir, 'MEMORY.md'), 'utf8'), before)
      stub('echo garbage-date')
      assert.match(attempt(''), /not YYYY-MM-DD/)
      assert.equal(readFileSync(join(dir, 'MEMORY.md'), 'utf8'), before)
      stub('true')
      assert.match(attempt(''), /returned nothing/)
      assert.equal(readFileSync(join(dir, 'MEMORY.md'), 'utf8'), before)
      for (const lying of ['2026-02-30', '2026-99-99']) {
        stub(`echo ${lying}`)
        assert.match(attempt(''), /not YYYY-MM-DD/)
        assert.equal(readFileSync(join(dir, 'MEMORY.md'), 'utf8'), before)
        assert.equal(existsSync(join(dir, 'memory', `${lying}.md`)), false)
      }
    } finally {
      process.env.PATH = oldPath
    }
  } finally {
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
  }
})

test('the addon renders the tool text', () => {
  const { renderMemory, renderSave } = loadNative()
  assert.equal(
    renderMemory(JSON.stringify({ ok: true, action: 'add', usage: { userUsed: 0, userCap: 2000, agentUsed: 19, agentCap: 4000 } })),
    'memory added (user 0/2000, agent 19/4000)',
  )
  assert.equal(renderMemory(JSON.stringify({ ok: true, action: 'query', entries: [] })), 'memory query: no matches')
  assert.equal(
    renderMemory(JSON.stringify({ ok: true, action: 'query', entries: [{ track: 'user', scope: 'user-global', text: 'A.' }] })),
    'memory query: [user/user-global] A.',
  )
  assert.equal(
    renderMemory(JSON.stringify({ ok: true, action: 'replace', entry: { track: 'agent', scope: 'workspace' }, usage: { userUsed: 0, userCap: 0, agentUsed: 1, agentCap: 2 } })),
    'memory replaced [agent/workspace] (user 0/0, agent 1/2)',
  )
  assert.equal(renderMemory(JSON.stringify({ ok: false, action: 'add', error: 'x' })), 'memory add failed: x')
  assert.equal(renderSave(JSON.stringify({ ok: true, row: 'r', part: 'p', day: 'd' })), 'saved: r · part p · d')
  assert.equal(renderSave(JSON.stringify({ ok: false, error: 'e' })), 'memory_save failed: e')
})

test('the cap refuses instead of growing; consolidate is the only route', () => {
  const { store, dir } = fresh()
  try {
    store.add('user', 'user-global', 'x'.repeat(1990), [], 'test', '/ws', '', 's1')
    assert.throws(() => store.add('user', 'user-global', 'y'.repeat(20), [], 'test', '/ws', '', 's1'), /budget/)
  } finally {
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
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
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
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
    try { rmSync(dir, { recursive: true, force: true }) } catch {}
  }
})
