// The repo-side tool's body, host-free. One call creates a fact the way the memory process says: a dated index row, its
// detail section, and the day's line.
import { resolve } from 'node:path'

export const SAVE_DESCRIPTION =
  'Save a fact to the workspace memory: a dated row in MEMORY.md (max 200 characters), optionally its section in a ' +
  'docs/memory-details part, and the line in memory/<date>.md. The pointer must be a detail part, a runbook, a skill or ' +
  'docs/hard-won.md. Refuses duplicates and rows that are too long.'

export const SAVE_PARAMETERS = Object.freeze({
  text: { type: 'string', required: true, description: 'The fact, one sentence, max 200 characters.' },
  pointer: { type: 'string', required: true, description: 'Where the detail lives: docs/memory-details/<part>.md, a runbook, a skill, or docs/hard-won.md.' },
  section: { type: 'string', description: 'MEMORY.md ### heading to insert under. Required when the file has no "## Subject facts": pass one of its "## " headings.' },
  part: { type: 'string', description: 'Detail part file name, e.g. 12-my-subject.md (with slug and detail).' },
  slug: { type: 'string', description: 'Section slug inside the part (no spaces).' },
  detail: { type: 'string', description: 'The long form for the part section.' },
  date: { type: 'string', description: 'YYYY-MM-DD (default: today, local).' },
})

export const SAVE_OUTPUT_SCHEMA = Object.freeze({
  type: 'object',
  additionalProperties: false,
  properties: {
    ok: { type: 'boolean', required: true },
    row: { type: 'string' },
    part: { type: 'string' },
    day: { type: 'string' },
    error: { type: 'string' },
  },
})

export const localDate = (d = new Date()) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`

export async function executeRepoAction(repoCtor, args, exec) {
  exec?.signal?.throwIfAborted?.()
  try {
    const root = resolve(exec?.agent?.session?.header?.cwd ?? process.cwd())
    const repo = new repoCtor(root)
    const written = repo.saveChecked(
      args.text ?? '',
      args.date ?? localDate(),
      args.pointer ?? '',
      args.section ?? null,
      args.part ?? null,
      args.slug ?? null,
      args.detail ?? null,
    )
    return { ok: true, row: written.row, part: written.part ?? '', day: written.day ?? '' }
  } catch (cause) {
    return { ok: false, error: String(cause?.message ?? cause) }
  }
}

export function renderRepoResult(_args, value) {
  if (!value.ok) return [{ type: 'text', text: `memory_save failed: ${value.error}` }]
  return [{ type: 'text', text: `saved: ${value.row}${value.part ? ` · part ${value.part}` : ''}${value.day ? ` · ${value.day}` : ''}` }]
}
