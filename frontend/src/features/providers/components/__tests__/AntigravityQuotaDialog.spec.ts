import { describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h } from 'vue'

import AntigravityQuotaDialog from '@/features/providers/components/AntigravityQuotaDialog.vue'
import type { QuotaStatusSnapshot, UpstreamMetadata } from '@/api/endpoints/types'

vi.mock('@/components/ui', async () => {
  const { defineComponent, h } = await import('vue')

  const passthrough = (name: string) => defineComponent({
    name,
    setup(_, { slots }) {
      return () => h('div', [
        slots.headerActions?.(),
        slots.default?.(),
        slots.footer?.(),
      ])
    },
  })

  return {
    Dialog: passthrough('DialogStub'),
    DropdownMenu: passthrough('DropdownMenuStub'),
    DropdownMenuTrigger: passthrough('DropdownMenuTriggerStub'),
    DropdownMenuContent: passthrough('DropdownMenuContentStub'),
    DropdownMenuItem: defineComponent({
      name: 'DropdownMenuItemStub',
      emits: ['select'],
      setup(_, { emit, slots }) {
        return () => h('button', { type: 'button', onClick: () => emit('select') }, slots.default?.())
      },
    }),
  }
})

vi.mock('@/components/ui/button.vue', async () => {
  const { defineComponent, h } = await import('vue')

  return {
    default: defineComponent({
      name: 'ButtonStub',
      setup(_, { attrs, slots }) {
        return () => h('button', { ...attrs, type: 'button' }, slots.default?.())
      },
    }),
  }
})

vi.mock('lucide-vue-next', async () => {
  const { defineComponent, h } = await import('vue')
  const Icon = defineComponent({
    name: 'IconStub',
    setup() {
      return () => h('span')
    },
  })

  return {
    BarChart3: Icon,
    Loader2: Icon,
    Play: Icon,
  }
})

vi.mock('@/api/endpoints/providers', () => ({
  testModel: vi.fn(),
}))

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({
    error: vi.fn(),
    success: vi.fn(),
  }),
}))

vi.mock('@/utils/errorParser', () => ({
  parseApiError: (value: unknown) => String(value),
}))

function mount(metadata: UpstreamMetadata, quotaSnapshot?: QuotaStatusSnapshot) {
  const root = document.createElement('div')
  document.body.appendChild(root)

  const app = createApp(defineComponent({
    setup() {
      return () => h(AntigravityQuotaDialog, {
        open: true,
        metadata,
        quotaSnapshot,
        keyName: 'Key-1',
      })
    },
  }))
  app.mount(root)

  return {
    root,
    unmount: () => {
      app.unmount()
      root.remove()
    },
  }
}

describe('AntigravityQuotaDialog', () => {
  it('renders grouped five-hour and weekly quota windows', () => {
    const { root, unmount } = mount({}, {
      code: 'ok',
      provider_type: 'antigravity',
      exhausted: false,
      windows: [{
        code: 'group:0:3p-5h',
        label: 'Claude and GPT models · 5 hour',
        scope: 'quota_group',
        used_ratio: 0.75,
        remaining_ratio: 0.25,
      }, {
        code: 'group:0:3p-weekly',
        label: 'Claude and GPT models · weekly',
        scope: 'quota_group',
        used_ratio: 0.2,
        remaining_ratio: 0.8,
      }],
    })
    const text = root.textContent || ''

    expect(text).toContain('Claude and GPT models · 5 hour')
    expect(text).toContain('25.0%')
    expect(text).toContain('Claude and GPT models · weekly')
    expect(text).toContain('80.0%')

    unmount()
  })

  it('hides quota buckets outside the shared pool summary families', () => {
    const rawIdentifier = 'RateLimitResetCredit_05cbb6eeeb9c81918e011d8300f9ebfb'
    const { root, unmount } = mount({
      antigravity: {
        quota_by_model: {
          [rawIdentifier]: {
            display_name: rawIdentifier,
            remaining_fraction: 0.25,
            used_percent: 75,
          },
        },
      },
    })

    expect(root.textContent).toContain('暂无配额数据')
    expect(root.textContent).not.toContain('Key-1')
    expect(root.textContent).not.toContain(rawIdentifier)
    expect(root.textContent).not.toContain('25.0%')

    unmount()
  })

  it('renders only the shared Gemini and Claude ChatGPT family summaries', () => {
    const { root, unmount } = mount({
      antigravity: {
        quota_by_model: {
          tab_flash_lite_preview: {
            display_name: 'Tab Flash Lite Preview',
            remaining_fraction: 0.01,
            used_percent: 99,
          },
          'gemini-3-flash-agent': {
            display_name: 'Gemini 3.5 Flash (High)',
            remaining_fraction: 0.9,
            used_percent: 10,
          },
          'gemini-pro-agent': {
            display_name: 'gemini-pro-agent',
            remaining_fraction: 0.95,
            used_percent: 5,
          },
          'gemini-3.5-flash-low': {
            display_name: 'Gemini 3.5 Flash (Medium)',
            remaining_fraction: 0.8,
            used_percent: 20,
          },
          'claude-opus-4-6-thinking': {
            display_name: 'Claude Opus 4.6 (Thinking)',
            remaining_fraction: 1,
            used_percent: 0,
          },
          chat_20706: {
            display_name: 'chat_20706',
            remaining_fraction: 0,
            used_percent: 100,
          },
        },
      },
    })
    const text = root.textContent || ''

    expect(text).not.toContain('gemini-pro-agent')
    expect(text).not.toContain('claude-opus-4-6-thinking')
    expect(text).toContain('Gemini额度')
    expect(text).toContain('80%–95%')
    expect(text).toContain('Claude & ChatGPT')
    expect(text).toContain('100%')
    expect(text).not.toContain('Gemini 3.1 Pro (High)')
    expect(text).not.toContain('Gemini 3.5 Flash (High)')
    expect(text).not.toContain('Gemini 3.5 Flash (Medium)')
    expect(text).not.toContain('Tab Flash Lite Preview')
    expect(text).not.toContain('chat_20706')

    unmount()
  })

  it('collapses duplicate model buckets into one family range', () => {
    const { root, unmount } = mount({
      antigravity: {
        quota_by_model: {
          'gemini-3.1-pro-high': {
            display_name: 'Gemini 3.1 Pro (High)',
            remaining_fraction: 0.95,
            used_percent: 5,
          },
          'gemini-pro-agent': {
            display_name: 'Gemini 3.1 Pro (High)',
            remaining_fraction: 0.4,
            used_percent: 60,
          },
          'gemini-3-flash-agent': {
            display_name: 'Gemini 3.5 Flash (High)',
            remaining_fraction: 0.9,
            used_percent: 10,
          },
        },
      },
    })
    const text = root.textContent || ''

    expect(text.match(/Gemini额度/g)).toHaveLength(1)
    expect(text).toContain('40%–90%')
    expect(text).not.toContain('95.0%')
    expect(text).not.toContain('Gemini 3.1 Pro (High)')
    expect(text).not.toContain('Gemini 3.5 Flash (High)')

    unmount()
  })
})
