import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

function readSource(path: string): string {
  return readFileSync(resolve(process.cwd(), path), 'utf8')
}

describe('ProviderFormDialog Codex WebSocket switch', () => {
  it('only displays and submits the switch for Codex providers', () => {
    const source = readSource('src/features/providers/components/ProviderFormDialog.vue')

    expect(source).toContain("v-if=\"form.provider_type === 'codex'\"")
    expect(source).toContain('Responses WebSocket 模式')
    expect(source).toContain('codex_responses_websocket_enabled')
    expect(source).toContain("form.value.provider_type === 'codex'")
  })
})
