// The memory tool's body and its result contract, host-free so it is unit-testable.
import { agentKeyOf, sessionOf, workspaceKeyOf } from './native.mjs'

export const MEMORY_DESCRIPTION =
  'Tier-0 memory: small, always-visible facts for this workspace. add/replace/remove/query. ' +
  'The store is capped (user 2000, agent 4000 characters); consolidate instead of growing it. ' +
  'Beliefs live in the repo (MEMORY.md, docs/); this store holds preferences and pointers.'

export const MEMORY_PARAMETERS = Object.freeze({
  action: { type: 'string', required: true, enum: ['add', 'replace', 'remove', 'query'], description: 'What to do.' },
  track: { type: 'string', enum: ['user', 'agent'], description: 'user = owner preferences, agent = workspace pointer.' },
  scope: { type: 'string', enum: ['user-global', 'workspace'], description: 'user-global everywhere; workspace only here.' },
  text: { type: 'string', description: 'Entry text (add/replace) or the search text (query).' },
  match: { type: 'string', description: 'Unique substring identifying the entry (replace/remove).' },
  tags: { type: 'array', items: { type: 'string' }, description: 'Short labels, not sentences.' },
  limit: { type: 'number', description: 'Max query hits (default 10).' },
})

const entryOf = (e) => ({ id: e.id, track: e.track, scope: e.scope, text: e.text })

/** The result schema a successful body must satisfy; also drives the tool's output contract. */
export const MEMORY_OUTPUT_SCHEMA = Object.freeze({
  type: 'object',
  additionalProperties: false,
  properties: {
    ok: { type: 'boolean', required: true },
    action: { type: 'string', required: true },
    id: { type: 'string' },
    entry: { type: 'object', additionalProperties: false, properties: { id: { type: 'string', required: true }, track: { type: 'string', required: true }, scope: { type: 'string', required: true }, text: { type: 'string', required: true } } },
    entries: { type: 'array', items: { type: 'object', additionalProperties: false, properties: { id: { type: 'string', required: true }, track: { type: 'string', required: true }, scope: { type: 'string', required: true }, text: { type: 'string', required: true } } } },
    usage: { type: 'object', additionalProperties: false, properties: { userUsed: { type: 'integer', required: true }, userCap: { type: 'integer', required: true }, agentUsed: { type: 'integer', required: true }, agentCap: { type: 'integer', required: true } } },
    error: { type: 'string' },
  },
})

export async function executeMemoryAction(store, args, exec) {
  exec?.signal?.throwIfAborted?.()
  try {
    switch (args.action) {
      case 'add': {
        const id = store.add(
          args.track ?? 'agent',
          args.scope ?? 'workspace',
          args.text ?? '',
          args.tags ?? [],
          'memory-tool',
          workspaceKeyOf(exec?.agent?.session?.header?.cwd),
          agentKeyOf(exec),
          sessionOf(exec),
        )
        return { ok: true, action: 'add', id, usage: store.usage() }
      }
      case 'replace':
        return { ok: true, action: 'replace', entry: entryOf(store.replace(args.match ?? '', args.text ?? '', sessionOf(exec))), usage: store.usage() }
      case 'remove':
        return { ok: true, action: 'remove', entry: entryOf(store.remove(args.match ?? '', sessionOf(exec))), usage: store.usage() }
      case 'query':
        return { ok: true, action: 'query', entries: store.query(args.text ?? '', Number(args.limit ?? 10), sessionOf(exec)).map(entryOf) }
      default:
        return { ok: false, action: String(args.action), error: `unknown action ${String(args.action)}` }
    }
  } catch (cause) {
    return { ok: false, action: String(args.action), error: String(cause?.message ?? cause) }
  }
}

/** One short line per outcome; the model reads it, the human does not. */
export function renderMemoryResult(_args, value) {
  if (!value.ok) return [{ type: 'text', text: `memory ${value.action} failed: ${value.error}` }]
  const budget = value.usage ? ` (user ${value.usage.userUsed}/${value.usage.userCap}, agent ${value.usage.agentUsed}/${value.usage.agentCap})` : ''
  switch (value.action) {
    case 'query':
      return [{ type: 'text', text: value.entries.length === 0 ? 'memory query: no matches' : `memory query: ${value.entries.map((e) => `[${e.track}/${e.scope}] ${e.text}`).join(' | ')}` }]
    case 'add':
      return [{ type: 'text', text: `memory added${budget}` }]
    case 'replace':
    case 'remove':
      return [{ type: 'text', text: `memory ${value.action}d [${value.entry.track}/${value.entry.scope}]${budget}` }]
    default:
      return [{ type: 'text', text: `memory ${value.action}: ok` }]
  }
}
