import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, type App } from 'vue'

import BatchAssignModelsDialog from '../BatchAssignModelsDialog.vue'
import { getGlobalModels } from '@/api/endpoints/global-models'
import { getProviderKeys, getProviderModels, type GlobalModelResponse } from '@/api/endpoints'

vi.mock('@/components/ui/dialog/Dialog.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      name: 'DialogStub',
      setup(_, { slots }) {
        return () => h('div', [slots.default?.(), slots.footer?.()])
      },
    }),
  }
})

vi.mock('@/components/ui/button.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      name: 'ButtonStub',
      setup(_, { slots }) {
        return () => h('button', slots.default?.())
      },
    }),
  }
})

vi.mock('@/components/ui/input.vue', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    default: defineComponent({
      name: 'InputStub',
      setup() {
        return () => h('input')
      },
    }),
  }
})

vi.mock('@/components/ui', async () => {
  const { defineComponent, h } = await import('vue')
  const passthrough = (name: string) => defineComponent({
    name,
    setup(_, { slots }) {
      return () => h('div', slots.default?.())
    },
  })
  return {
    DropdownMenu: passthrough('DropdownMenuStub'),
    DropdownMenuTrigger: passthrough('DropdownMenuTriggerStub'),
    DropdownMenuContent: passthrough('DropdownMenuContentStub'),
    DropdownMenuItem: passthrough('DropdownMenuItemStub'),
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
    Check: Icon,
    Layers: Icon,
    ListChecks: Icon,
    Loader2: Icon,
    Search: Icon,
  }
})

vi.mock('@/api/endpoints/global-models', () => ({
  getGlobalModels: vi.fn(),
}))

vi.mock('@/api/endpoints', () => ({
  batchAssignModelsToProvider: vi.fn(),
  deleteModel: vi.fn(),
  getProviderKeys: vi.fn(),
  getProviderModels: vi.fn(),
}))

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({
    error: vi.fn(),
    success: vi.fn(),
    warning: vi.fn(),
  }),
}))

vi.mock('@/composables/useConfirm', () => ({
  useConfirm: () => ({ confirmWarning: vi.fn() }),
}))

vi.mock('../../composables/useUpstreamModelsCache', () => ({
  useUpstreamModelsCache: () => ({ fetchModels: vi.fn() }),
}))

const mountedApps: Array<{ app: App, root: HTMLElement }> = []

afterEach(() => {
  for (const { app, root } of mountedApps.splice(0)) {
    app.unmount()
    root.remove()
  }
  vi.resetAllMocks()
})

describe('BatchAssignModelsDialog', () => {
  it('在初始打开时加载并显示全局模型', async () => {
    const globalModel = {
      id: 'global-gpt-5',
      name: 'gpt-5',
      display_name: 'GPT 5',
      is_active: false,
      default_tiered_pricing: { tiers: [] },
      created_at: '2026-07-27T00:00:00Z',
    } satisfies GlobalModelResponse
    vi.mocked(getGlobalModels).mockResolvedValue({ models: [globalModel], total: 1 })
    vi.mocked(getProviderModels).mockResolvedValue([])
    vi.mocked(getProviderKeys).mockResolvedValue([])

    const root = document.createElement('div')
    document.body.appendChild(root)
    const app = createApp(defineComponent({
      setup() {
        return () => h(BatchAssignModelsDialog, {
          open: true,
          providerId: 'provider-1',
          providerName: '测试提供商',
        })
      },
    }))
    app.mount(root)
    mountedApps.push({ app, root })

    await vi.waitFor(() => {
      expect(getGlobalModels).toHaveBeenCalledWith({ limit: 1000 })
      expect(root.textContent).toContain('GPT 5')
    })
    expect(root.textContent).not.toContain('暂无可用全局模型')
  })
})
