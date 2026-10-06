import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, ref, type App } from 'vue'
import { createMemoryHistory, createRouter, RouterView, type LocationQueryRaw } from 'vue-router'
import ProviderSchedulingView from '../ProviderSchedulingView.vue'
import { createEmptyRoutingGroupConfig, type RoutingGroupConfig } from '@/features/routing/utils/routingPolicy'
import type { RoutingGroupRecord, RoutingGroupUpdateRequest } from '@/api/routing-profiles'

const routingApi = vi.hoisted(() => ({ listRoutingGroups: vi.fn(), updateRoutingGroup: vi.fn(), createRoutingGroup: vi.fn(), deleteRoutingGroup: vi.fn() }))
const confirm = vi.hoisted(() => vi.fn())
const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }))
const inspectProvider = vi.hoisted(() => vi.fn())
const contextChange = vi.hoisted(() => vi.fn())
vi.mock('@/api/routing-profiles', () => routingApi)
vi.mock('@/api/global-models', () => ({ getGlobalModels: vi.fn().mockResolvedValue({ models: [] }) }))
vi.mock('@/composables/useToast', () => ({ useToast: () => toast }))
vi.mock('@/composables/useConfirm', () => ({ useConfirm: () => ({ confirm }) }))
vi.mock('@/utils/logger', () => ({ log: { error: vi.fn(), warn: vi.fn() } }))
vi.mock('@/features/providers/composables/useSchedulingProviderBalance', () => ({ provideSchedulingProviderBalance: vi.fn() }))
vi.mock('../ProviderSchedulingStatus.vue', () => ({ default: { render: () => null } }))
vi.mock('@/features/routing/components', async () => {
  const { defineComponent, h } = await import('vue')
  return {
    RoutingFailoverPolicyEditor: defineComponent({ setup(_, { expose }) { expose({ commitJsonDrafts: () => true }); return () => null } }),
    RoutingSchedulingPolicyEditor: defineComponent({
      props: { refreshRevision: Number, layout: String },
      emits: ['inspect-provider'],
      setup(props, { emit, slots }) {
        return () => h('div', [
          slots['toolbar-leading']?.(),
          h('button', { 'aria-label': '查看示例提供商', 'data-revision': props.refreshRevision, 'data-layout': props.layout, onClick: () => emit('inspect-provider', 'provider-a') }, '示例提供商'),
          slots['toolbar-actions']?.(),
        ])
      },
    }),
  }
})

const mounted: Array<{ app: App; root: HTMLElement }> = []
const originalScrollIntoView = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollIntoView')
function group(id: string, overrides: Partial<RoutingGroupRecord> = {}): RoutingGroupRecord {
  return { id, name: id, enabled: true, is_system_default: false, sort_order: 0, config_json: createEmptyRoutingGroupConfig(), version: 1, created_at: 1, updated_at: 1, ...overrides }
}
async function flush() {
  await nextTick()
  await new Promise(resolve => setTimeout(resolve, 0))
  await nextTick()
}
async function mountWorkspace(query: LocationQueryRaw = {}, items = [group('first'), group('default', { is_system_default: true }), group('last')]) {
  routingApi.listRoutingGroups.mockResolvedValue({ items, total: items.length })
  routingApi.updateRoutingGroup.mockImplementation(async (id: string, payload: RoutingGroupUpdateRequest) => ({ ...items.find(item => item.id === id), ...payload, version: 2 }))
  routingApi.createRoutingGroup.mockImplementation(async payload => group('created', payload))
  const providerRevision = ref(0)
  const workspace = ref<{ updateDraftConfig: (config: RoutingGroupConfig) => void; refreshGroups: () => Promise<void>; ensureSaved: () => Promise<boolean> } | null>(null)
  const Providers = defineComponent({
    setup() {
      return () => h(ProviderSchedulingView, { ref: workspace, providerRevision: providerRevision.value, onInspectProvider: inspectProvider, onContextChange: contextChange }, { default: () => h('div', { 'data-testid': 'provider-directory' }, '提供商目录') })
    },
  })
  const router = createRouter({ history: createMemoryHistory(), routes: [
    { path: '/providers', name: 'ProviderManagement', component: Providers },
    { path: '/other', name: 'Other', component: { render: () => h('div', '其他页面') } },
  ] })
  await router.push({ name: 'ProviderManagement', query })
  await router.isReady()
  const root = document.createElement('div')
  document.body.appendChild(root)
  const app = createApp({ render: () => h(RouterView) })
  app.use(router)
  app.mount(root)
  mounted.push({ app, root })
  await flush()
  return { root, router, providerRevision, workspace }
}
function element<T extends HTMLElement>(root: HTMLElement, selector: string): T {
  const found = root.querySelector<T>(selector) ?? document.body.querySelector<T>(selector)
  if (!found) throw new Error(`Missing ${selector}`)
  return found
}
function button(root: HTMLElement, label: string) {
  return element<HTMLButtonElement>(root, `button[aria-label="${label}"]`)
}
function selector(root: HTMLElement) {
  return element<HTMLButtonElement>(root, 'button[role="combobox"][aria-label="当前调度策略"]')
}
async function openGroupOptions(root: HTMLElement) {
  selector(root).focus()
  selector(root).dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }))
  await flush()
  return [...document.querySelectorAll<HTMLElement>('[role="listbox"] [role="option"]')]
}
async function chooseGroup(root: HTMLElement, label: string) {
  const option = (await openGroupOptions(root)).find(item => item.textContent?.trim() === label)
  if (!option) throw new Error(`Missing group option: ${label}`)
  option.focus()
  option.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
  await flush()
}
async function editName(root: HTMLElement, name: string) {
  const input = element<HTMLInputElement>(root, '[aria-label="策略名称"]')
  input.value = name
  input.dispatchEvent(new Event('input', { bubbles: true }))
  await nextTick()
}

beforeEach(() => {
  vi.clearAllMocks()
  confirm.mockResolvedValue(false)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { value: vi.fn(), configurable: true })
})
afterEach(() => {
  for (const { app, root } of mounted.splice(0)) { app.unmount(); root.remove() }
  vi.unstubAllGlobals()
  if (originalScrollIntoView) Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', originalScrollIntoView)
  else Reflect.deleteProperty(HTMLElement.prototype, 'scrollIntoView')
})

describe('ProviderSchedulingView workspace navigation', () => {
  it('opens the system default immediately and forwards provider inspection and refreshes', async () => {
    const { root, providerRevision } = await mountWorkspace()
    expect(selector(root).textContent?.trim()).toBe('default · 默认')
    expect(button(root, '保存调度').disabled).toBe(true)
    expect(root.querySelector('[data-testid="provider-directory"]')).not.toBeNull()
    expect(document.querySelector('[role="dialog"]')).toBeNull()
    expect(button(root, '查看示例提供商').dataset.layout).toBe('config-only')
    const directory = element(root, '[data-testid="provider-directory"]')
    const combined = element(root, '[aria-label="策略分组与调度配置"]')
    expect(combined.contains(selector(root))).toBe(true)
    expect(combined.contains(button(root, '查看示例提供商'))).toBe(true)
    expect(combined.contains(directory)).toBe(false)
    const header = element(combined, 'h3').parentElement!
    expect(header.contains(selector(root))).toBe(true)
    expect(header.contains(button(root, '新建策略'))).toBe(true)
    expect(header.contains(button(root, '删除策略'))).toBe(true)
    expect(header.contains(button(root, '保存调度'))).toBe(true)
    expect(header.contains(button(root, '设为系统默认'))).toBe(true)
    const headerButtons = [...header.querySelectorAll<HTMLButtonElement>('button')]
    expect(headerButtons[headerButtons.indexOf(button(root, '保存调度')) - 1]).toBe(button(root, '设为系统默认'))
    expect(button(root, '设为系统默认').textContent?.trim()).toBe('')
    expect(button(root, '设为系统默认').getAttribute('aria-pressed')).toBe('true')
    expect(button(root, '保存调度').textContent?.trim()).toBe('')
    expect(button(root, '新建策略').textContent?.trim()).toBe('')
    expect(button(root, '删除策略').textContent?.trim()).toBe('')
    expect(combined.querySelector('[role="status"]')).toBeNull()
    expect(combined.querySelector('[aria-label="撤销修改"]')).toBeNull()
    expect(button(root, '查看示例提供商').compareDocumentPosition(directory) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(button(root, '故障转移').getAttribute('aria-expanded')).toBe('false')
    expect(button(root, '高级设置').getAttribute('aria-expanded')).toBe('false')
    expect(combined.contains(button(root, '故障转移'))).toBe(true)
    expect(combined.contains(button(root, '高级设置'))).toBe(true)
    expect(root.querySelector('[aria-label="分组管理"]')).toBeNull()
    expect(root.querySelector('[aria-label="管理策略"]')).toBeNull()
    const controlOrder = [
      selector(root),
      element(root, '[aria-label="策略名称"]'),
      button(root, '启用策略'),
      element(root, '[aria-label="策略描述"]'),
      button(root, '查看示例提供商'),
      button(root, '高级设置'),
      button(root, '故障转移'),
    ]
    for (let index = 1; index < controlOrder.length; index += 1) {
      expect(controlOrder[index - 1].compareDocumentPosition(controlOrder[index]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    }
    button(root, '查看示例提供商').click()
    expect(inspectProvider).toHaveBeenCalledWith('provider-a')
    await editName(root, '保持草稿')
    providerRevision.value += 1
    await nextTick()
    expect(button(root, '查看示例提供商').dataset.revision).toBe('1')
    expect(element<HTMLInputElement>(root, '[aria-label="策略名称"]').value).toBe('保持草稿')
    expect(button(root, '保存调度').disabled).toBe(false)
  })

  it('keeps the route, dropdown and draft unchanged when a strategy switch is cancelled', async () => {
    const { root, router } = await mountWorkspace()
    await editName(root, '未保存名称')
    await chooseGroup(root, 'last')
    expect(confirm).toHaveBeenCalledOnce()
    expect(router.currentRoute.value.query.group).toBeUndefined()
    expect(selector(root).textContent?.trim()).toBe('default · 默认')
    expect(element<HTMLInputElement>(root, '[aria-label="策略名称"]').value).toBe('未保存名称')
    expect(routingApi.updateRoutingGroup).not.toHaveBeenCalled()
    const options = await openGroupOptions(root)
    expect(options.find(option => option.getAttribute('aria-selected') === 'true')?.textContent?.trim()).toBe('default · 默认')
    expect(options.find(option => option.textContent?.trim() === 'last')?.getAttribute('aria-selected')).toBe('false')
  })

  it('switches only after discard is confirmed and resets the saved state', async () => {
    const { root, router } = await mountWorkspace()
    await editName(root, '未保存名称')
    confirm.mockResolvedValue(true)
    await chooseGroup(root, 'last')
    expect(router.currentRoute.value.query.group).toBe('last')
    expect(selector(root).textContent?.trim()).toBe('last')
    expect(element<HTMLInputElement>(root, '[aria-label="策略名称"]').value).toBe('last')
    expect(button(root, '保存调度').disabled).toBe(true)
  })

  it('protects leaving the provider page while unrelated query changes preserve the directory', async () => {
    const { root, router } = await mountWorkspace()
    await editName(root, '需要保护')
    await router.push({ name: 'ProviderManagement', query: { search: 'sample' } })
    expect(confirm).not.toHaveBeenCalled()
    await router.push({ name: 'Other' })
    expect(router.currentRoute.value.name).toBe('ProviderManagement')
    expect(confirm).toHaveBeenCalledOnce()
    confirm.mockResolvedValue(true)
    await router.push({ name: 'Other' })
    expect(router.currentRoute.value.name).toBe('Other')
  })

  it('blocks navigation while saving and restores a clean snapshot after success', async () => {
    const { root, router } = await mountWorkspace()
    await editName(root, '已保存名称')
    let resolveSave!: (value: RoutingGroupRecord) => void
    routingApi.updateRoutingGroup.mockReturnValue(new Promise<RoutingGroupRecord>(resolve => { resolveSave = resolve }))
    button(root, '保存调度').click()
    await nextTick()
    await router.push({ name: 'Other' })
    expect(router.currentRoute.value.name).toBe('ProviderManagement')
    expect(confirm).not.toHaveBeenCalled()
    expect(toast.error).toHaveBeenCalledWith('正在保存调度设置，请稍候再切换')
    resolveSave(group('default', { name: '已保存名称', is_system_default: true }))
    await flush()
    expect(button(root, '保存调度').disabled).toBe(true)
    await router.push({ name: 'Other' })
    expect(router.currentRoute.value.name).toBe('Other')
  })

  it('retains edits after a failed save and can retry without changing route', async () => {
    const { root, router } = await mountWorkspace()
    await editName(root, '失败后保留')
    routingApi.updateRoutingGroup.mockRejectedValue(new Error('offline'))
    button(root, '保存调度').click()
    await flush()
    expect(element<HTMLInputElement>(root, '[aria-label="策略名称"]').value).toBe('失败后保留')
    expect(button(root, '保存调度').disabled).toBe(false)
    routingApi.updateRoutingGroup.mockResolvedValue(group('default', { name: '失败后保留', is_system_default: true }))
    button(root, '保存调度').click()
    await flush()
    expect(routingApi.updateRoutingGroup).toHaveBeenCalledTimes(2)
    expect(routingApi.updateRoutingGroup).toHaveBeenLastCalledWith('default', expect.objectContaining({ name: '失败后保留', expected_version: 1 }))
    expect(element<HTMLInputElement>(root, '[aria-label="策略名称"]').value).toBe('失败后保留')
    expect(button(root, '保存调度').disabled).toBe(true)
    expect(router.currentRoute.value.query.group).toBeUndefined()
  })

  it('opens creation independently and cancellation preserves the selected group and draft', async () => {
    const { root, router } = await mountWorkspace()
    await editName(root, '保留的分组草稿')
    button(root, '新建策略').click()
    await nextTick()
    expect(selector(root).textContent?.trim()).toBe('default · 默认')
    expect(router.currentRoute.value.query.group).toBeUndefined()
    expect(document.querySelector('[role="dialog"][aria-label="新建策略分组"]')).not.toBeNull()
    button(root, '取消新建分组').click()
    await nextTick()
    expect(button(root, '保存调度').disabled).toBe(false)
    expect(contextChange.mock.lastCall?.[0].groupName).toBe('保留的分组草稿')
    expect(routingApi.createRoutingGroup).not.toHaveBeenCalled()
    expect(confirm).not.toHaveBeenCalled()
  })

  it('creates in a dialog and switches only after the request succeeds', async () => {
    const { root, router } = await mountWorkspace({ group: 'new' })
    expect(selector(root).textContent?.trim()).toBe('default · 默认')
    expect(router.currentRoute.value.query.group).toBe('default')
    const name = element<HTMLInputElement>(root, '[aria-label="新分组名称"]')
    name.value = '新策略'
    name.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()
    let finish!: (group: RoutingGroupRecord) => void
    routingApi.createRoutingGroup.mockReturnValue(new Promise(resolve => { finish = resolve }))
    button(root, '创建策略分组').click()
    await nextTick()
    expect(selector(root).textContent?.trim()).toBe('default · 默认')
    finish(group('created', { name: '新策略' }))
    await flush()
    expect(routingApi.createRoutingGroup).toHaveBeenCalledWith(expect.objectContaining({ name: '新策略', config_json: expect.objectContaining({ disabled_providers: [] }) }))
    expect(router.currentRoute.value.query.group).toBe('created')
    expect(selector(root).textContent?.trim()).toBe('新策略')
    expect(button(root, '保存调度').disabled).toBe(true)
    expect(confirm).not.toHaveBeenCalled()
  })

  it('saves strategy metadata with scheduling and updates the default marker', async () => {
    const { root } = await mountWorkspace({ group: 'last' })
    await editName(root, '主要策略')
    expect(button(root, '设为系统默认').getAttribute('aria-pressed')).toBe('false')
    button(root, '设为系统默认').click()
    await nextTick()
    expect(button(root, '设为系统默认').getAttribute('aria-pressed')).toBe('true')
    expect(routingApi.updateRoutingGroup).not.toHaveBeenCalled()
    button(root, '保存调度').click()
    await flush()
    expect(routingApi.updateRoutingGroup).toHaveBeenCalledWith('last', expect.objectContaining({ name: '主要策略', is_system_default: true, config_json: expect.any(Object) }))
    expect(selector(root).textContent?.trim()).toBe('主要策略 · 默认')
    const options = await openGroupOptions(root)
    expect(options.find(option => option.getAttribute('aria-selected') === 'true')?.textContent?.trim()).toBe('主要策略 · 默认')
    expect(options.find(option => option.textContent?.trim() === 'default')?.getAttribute('aria-selected')).toBe('false')
    expect(button(root, '设为系统默认').getAttribute('aria-pressed')).toBe('true')
    expect(button(root, '保存调度').disabled).toBe(true)
  })

  it('accepts directory changes and saves membership with the draft base version', async () => {
    const { root, workspace } = await mountWorkspace()
    const config = createEmptyRoutingGroupConfig()
    config.disabled_providers = ['provider-a']
    workspace.value?.updateDraftConfig(config)
    await nextTick()
    expect(contextChange.mock.lastCall?.[0].config.disabled_providers).toEqual(['provider-a'])
    confirm.mockResolvedValue(true)
    expect(await workspace.value?.ensureSaved()).toBe(true)
    expect(routingApi.updateRoutingGroup).toHaveBeenCalledWith('default', expect.objectContaining({ expected_version: 1, config_json: expect.objectContaining({ disabled_providers: ['provider-a'] }) }))
    await flush()
    expect(button(root, '保存调度').disabled).toBe(true)
  })

  it('preserves a stale draft version after refreshing groups and shows a save conflict', async () => {
    const { root, workspace } = await mountWorkspace()
    await editName(root, '我的修改')
    routingApi.listRoutingGroups.mockResolvedValue({ items: [group('default', { is_system_default: true, version: 9, name: '其他人的修改' })] })
    await workspace.value?.refreshGroups()
    expect(element<HTMLInputElement>(root, '[aria-label="策略名称"]').value).toBe('我的修改')
    routingApi.updateRoutingGroup.mockRejectedValue({ response: { status: 409 } })
    button(root, '保存调度').click()
    await flush()
    expect(routingApi.updateRoutingGroup).toHaveBeenCalledWith('default', expect.objectContaining({ expected_version: 1 }))
    expect(button(root, '保存调度').disabled).toBe(false)
    expect(toast.error).toHaveBeenCalledWith(expect.stringContaining('其他操作中更新'))
    confirm.mockResolvedValue(true)
    button(root, '重新加载分组').click()
    await flush()
    expect(element<HTMLInputElement>(root, '[aria-label="策略名称"]').value).toBe('其他人的修改')
    await editName(root, '基于新版修改')
    routingApi.updateRoutingGroup.mockResolvedValue(group('default', { version: 10, name: '基于新版修改', is_system_default: true }))
    button(root, '保存调度').click()
    await flush()
    expect(routingApi.updateRoutingGroup).toHaveBeenLastCalledWith('default', expect.objectContaining({ expected_version: 9 }))
    expect(button(root, '保存调度').disabled).toBe(true)
  })

  it('warns before a tab refresh only when there are unsaved changes', async () => {
    const { root } = await mountWorkspace()
    const clean = new Event('beforeunload', { cancelable: true })
    window.dispatchEvent(clean)
    expect(clean.defaultPrevented).toBe(false)
    await editName(root, '刷新保护')
    const dirty = new Event('beforeunload', { cancelable: true })
    window.dispatchEvent(dirty)
    expect(dirty.defaultPrevented).toBe(true)
  })
})
