import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const frontendRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const moduleRoot = resolve(frontendRoot, '..', 'aether-vscodex')
const vueBuild = resolve(moduleRoot, 'web', 'dist')
const destination = resolve(frontendRoot, 'public', 'aether-vscodex')

if (!existsSync(resolve(vueBuild, 'index.html'))) {
  throw new Error(`aether-vscodex Vue build was not found at ${vueBuild}`)
}

rmSync(destination, { recursive: true, force: true })
mkdirSync(destination, { recursive: true })
cpSync(vueBuild, destination, { recursive: true })
