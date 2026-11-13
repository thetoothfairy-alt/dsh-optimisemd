import type { Context } from '@deepseek-ai/cordis'

export interface Config {
  /** SQLite file; empty = $DSH_HOME/dsh-memory/memory.db */
  dbPath?: string
  /** Characters allowed in user/user-global (default 2000) */
  userCap?: number
  /** Characters allowed in agent/workspace (default 4000) */
  agentCap?: number
  /** systemPrompt section order (default -50) */
  snapshotOrder?: number
}

export declare const name: string
export declare const inject: string[]
export declare const Config: unknown
export declare function apply(ctx: Context, config?: Config): void
