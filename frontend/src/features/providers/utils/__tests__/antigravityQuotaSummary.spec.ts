import { describe, expect, it } from 'vitest'

import { summarizeAntigravityQuotaItems } from '@/features/providers/utils/antigravityQuota'

describe('summarizeAntigravityQuotaItems', () => {
  it('groups model families without collapsing their independent quota values', () => {
    const items = summarizeAntigravityQuotaItems([
      { model: 'claude-opus-4-6-thinking', label: 'Claude Opus', remainingPercent: 100, resetSeconds: 60 },
      { model: 'claude-sonnet-4-6', label: 'Claude Sonnet', remainingPercent: 82, resetSeconds: 120 },
      { model: 'gemini-3.1-pro-high', label: 'Gemini Pro', remainingPercent: 90.6, resetSeconds: 180 },
      { model: 'gemini-3-flash-agent', label: 'Gemini Flash', remainingPercent: 95, resetSeconds: 240 },
      { model: 'gpt-oss-120b-medium', label: 'GPT-OSS', remainingPercent: 100, resetSeconds: 300 },
      { model: 'tab_flash_lite_preview', label: 'Tab', remainingPercent: 76, resetSeconds: 360 },
      { model: 'chat_20706', label: 'Chat', remainingPercent: 64, resetSeconds: 420 },
    ])

    expect(items.map(item => [item.label, item.remainingPercent, item.detail])).toEqual([
      ['Gemini额度', 90.6, '90.6–95'],
      ['Claude额度', 82, '82–100'],
    ])
    expect(items[1]?.model).toBe('claude-sonnet-4-6')
  })
})