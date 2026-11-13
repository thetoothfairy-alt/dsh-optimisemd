// dsh-optimisemd — tier-0 memory for DSH: a Rust store behind a native addon, plus the frozen snapshot the
// session reads at its first assembly. The host surface is small on purpose: tools + systemPrompt.
// Everything that can be wrong (caps, unique-match, audit) lives in Rust, and the host-free halves live
// in lib/ so they can be tested without a harness.

import { defineTool } from '@deepseek-ai/dsh-tools'
import Schema from '@deepseek-ai/schemastery'
import { dbPathOf, loadNative } from './lib/native.mjs'
import { MEMORY_DESCRIPTION, MEMORY_OUTPUT_SCHEMA, MEMORY_PARAMETERS, executeMemoryAction, renderMemoryResult } from './lib/action.mjs'

export const name = 'memory'
export const inject = ['tools', 'systemPrompt']

export const DEFAULT_BUDGETS = Object.freeze({ user: 2000, agent: 4000 })
export const DEFAULT_SNAPSHOT_ORDER = -50

export const Config = Schema.object({
  dbPath: Schema.string().default('').description('SQLite file; empty = $DSH_HOME/dsh-optimisemd/memory.db'),
  userCap: Schema.number().default(DEFAULT_BUDGETS.user).description('Characters for user/user-global'),
  agentCap: Schema.number().default(DEFAULT_BUDGETS.agent).description('Characters for agent/workspace'),
  snapshotOrder: Schema.number().default(DEFAULT_SNAPSHOT_ORDER).description('systemPrompt section order'),
})

export function makeMemoryTool(store) {
  return defineTool({
    name: 'memory',
    description: MEMORY_DESCRIPTION,
    parameters: MEMORY_PARAMETERS,
    output: { schema: MEMORY_OUTPUT_SCHEMA, render: renderMemoryResult },
    execute: (args, exec) => executeMemoryAction(store, args, exec),
  })
}

export function apply(ctx, config = {}) {
  const native = loadNative()
  const budgets = { user: config.userCap ?? DEFAULT_BUDGETS.user, agent: config.agentCap ?? DEFAULT_BUDGETS.agent }
  const store = new native.MemoryStore(dbPathOf(config), budgets.user, budgets.agent)
  const snapshots = new WeakMap()

  ctx.tools.register(makeMemoryTool(store))

  ctx.systemPrompt.section({
    name: 'dsh-optimisemd:memory',
    order: config.snapshotOrder ?? DEFAULT_SNAPSHOT_ORDER,
    text: (assemble) => {
      const session = assemble?.agent?.session
      if (session == null) return ''
      let frozen = snapshots.get(session)
      if (frozen === undefined) {
        frozen = store.snapshot(String(session?.header?.cwd ?? process.cwd()), String(session?.header?.agentPreset ?? ''))
        snapshots.set(session, frozen)
      }
      return frozen
    },
  })
}
