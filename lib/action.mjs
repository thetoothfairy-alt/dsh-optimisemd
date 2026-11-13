// The memory tool's body, host-free so it is unit-testable without the harness.
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

export async function executeMemoryAction(store, args, exec) {
  exec?.signal?.throwIfAborted?.()
  try {
    switch (args.action) {
      case 'add':
        return {
          ok: true,
          action: 'add',
          id: store.add(
            args.track ?? 'agent',
            args.scope ?? 'workspace',
            args.text ?? '',
            args.tags ?? [],
            'memory-tool',
            workspaceKeyOf(exec?.agent?.session?.header?.cwd),
            agentKeyOf(exec),
            sessionOf(exec),
          ),
          usage: store.usage(),
        }
      case 'replace':
        return { ok: true, action: 'replace', entry: store.replace(args.match ?? '', args.text ?? '', sessionOf(exec)), usage: store.usage() }
      case 'remove':
        return { ok: true, action: 'remove', entry: store.remove(args.match ?? '', sessionOf(exec)), usage: store.usage() }
      case 'query':
        return { ok: true, action: 'query', hits: store.query(args.text ?? '', Number(args.limit ?? 10), sessionOf(exec)) }
      default:
        return { ok: false, error: `unknown action ${String(args.action)}` }
    }
  } catch (cause) {
    return { ok: false, error: String(cause?.message ?? cause) }
  }
}
