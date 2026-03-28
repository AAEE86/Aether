import { describe, expect, it } from 'vitest'

import { buildPoolAdvancedHighlights } from '@/features/pool/utils/poolAdvancedDialog'

describe('poolAdvancedDialog', () => {
  it('returns enabled probing and cost summaries for pool settings', () => {
    const items = buildPoolAdvancedHighlights(
      {
        health_policy_enabled: true,
        probing_enabled: true,
        probing_interval_minutes: 15,
        cost_window_seconds: 18_000,
        batch_concurrency: 12,
      },
      null,
      false,
    )

    expect(items).toEqual([
      { key: 'health', label: '健康策略', value: '已开启', tone: 'success' },
      { key: 'probing', label: '主动探测', value: '15 分钟/次', tone: 'default' },
      { key: 'cost', label: '成本窗口', value: '5 小时', tone: 'default' },
      { key: 'batch', label: '批量并发', value: '12 路', tone: 'default' },
    ])
  })

  it('returns disabled and unset summaries when pool options are absent', () => {
    const items = buildPoolAdvancedHighlights(
      {
        health_policy_enabled: false,
        probing_enabled: false,
      },
      null,
      false,
    )

    expect(items).toEqual([
      { key: 'health', label: '健康策略', value: '已关闭', tone: 'warning' },
      { key: 'probing', label: '主动探测', value: '未启用', tone: 'muted' },
      { key: 'cost', label: '成本窗口', value: '未配置', tone: 'muted' },
      { key: 'batch', label: '批量并发', value: '默认', tone: 'muted' },
    ])
  })

  it('includes claude code session summary when provider type is claude_code', () => {
    const items = buildPoolAdvancedHighlights(
      {
        health_policy_enabled: true,
        probing_enabled: false,
      },
      {
        max_sessions: 6,
        session_idle_timeout_minutes: 8,
      },
      true,
    )

    expect(items.at(-1)).toEqual({
      key: 'claude-session',
      label: '会话控制',
      value: '最多 6 会话 / 空闲 8 分钟',
      tone: 'default',
    })
  })
})
