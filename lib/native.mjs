// Host-free half of the plugin: locate the addon, resolve the database path, describe its target.
import { createRequire } from 'node:module'
import { homedir } from 'node:os'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const require = createRequire(import.meta.url)
const here = dirname(dirname(fileURLToPath(import.meta.url)))

/** Platform-tagged addon filename; the release workflow attaches one per target. */
export function nativeFile(platform = process.platform, arch = process.arch) {
  const os = { linux: 'linux', darwin: 'darwin', win32: 'win32' }[platform] ?? platform
  return `dsh-optimisemd.${os}-${arch}.node`
}

/** Load the addon, or explain exactly which file is missing and how to build it. */
export function loadNative({ base = here, platform, arch } = {}) {
  const file = join(base, 'native', nativeFile(platform, arch))
  try {
    return require(file)
  } catch (cause) {
    const error = new Error(
      `dsh-optimisemd: native addon not found at ${file}. Build it with \`pnpm build\` ` +
        `(cargo build --release, then copy target/release/liboptimisemd_node.so to that path).`,
    )
    error.cause = cause
    throw error
  }
}

export function dbPathOf({ dbPath } = {}) {
  if (dbPath) return isAbsolute(dbPath) ? dbPath : resolve(dbPath)
  const home = process.env.DSH_HOME || join(homedir(), '.dsh')
  return join(home, 'dsh-optimisemd', 'memory.db')
}

export const workspaceKeyOf = (cwd) => resolve(cwd ?? process.cwd())
export const sessionOf = (exec) => String(exec?.agent?.session?.id ?? '')
export const agentKeyOf = (exec) => String(exec?.agent?.session?.header?.agentPreset ?? '')
