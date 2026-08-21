import { describe, expect, it } from 'vitest'

import { getEndpointTooltip } from '../useEndpointStatus'

describe('getEndpointTooltip', () => {
  it('formats the internal Codex Live id at the user-visible tooltip boundary', () => {
    const tooltip = getEndpointTooltip({
      api_format: 'codex:live',
      health_score: 1,
      is_active: true,
      total_keys: 1,
      active_keys: 1,
    }, 'zh-CN')

    expect(tooltip).toBe('OpenAI Live: 健康度 100%')
    expect(tooltip).not.toContain('codex:live')
  })
})
