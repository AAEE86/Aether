import { createApp, nextTick } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { setI18nLocale } from '@/i18n'
import UserUsageStats from '../users/UserUsageStats.vue'

const api = vi.hoisted(() => ({
  users: vi.fn(), groups: vi.fn(), members: vi.fn(),
  userLeaderboard: vi.fn(), groupLeaderboard: vi.fn(), summary: vi.fn(), series: vi.fn(),
}))
vi.mock('@/api/users', () => ({ usersApi: { getAllUsers: api.users, listUserGroups: api.groups, listUserGroupMembers: api.members } }))
vi.mock('@/api/admin', () => ({ adminApi: { getLeaderboardUsers: api.userLeaderboard, getLeaderboardUserGroups: api.groupLeaderboard, getTimeSeries: api.series } }))
vi.mock('@/api/usage', () => ({ usageApi: { getUsageStats: api.summary } }))
vi.mock('@/components/charts/LineChart.vue', () => ({ default: { render: () => null } }))
vi.mock('@/components/common', () => ({ EmptyState: { render: () => null }, LoadingState: { render: () => null }, TimeRangePicker: { render: () => null } }))
vi.mock('@/components/ui', async importOriginal => {
  const original = await importOriginal<object>()
  const { defineComponent, h } = await import('vue')
  return {
    ...original,
    Select: defineComponent({
      props: { modelValue: String }, emits: ['update:modelValue'],
      setup: (props, { slots, emit }) => () => h('select', {
        value: props.modelValue,
        onChange: (event: Event) => emit('update:modelValue', (event.target as HTMLSelectElement).value),
      }, slots.default?.()),
    }),
    SelectTrigger: { render: () => null },
    SelectContent: defineComponent({ inheritAttrs: false, setup: (_, { slots }) => () => slots.default?.() }),
    SelectItem: defineComponent({ props: { value: String }, setup: (props, { slots }) => () => h('option', { value: props.value }, slots.default?.()) }),
  }
})

let unmount = () => {}
async function settle() {
  for (let index = 0; index < 10; index += 1) await Promise.resolve()
  await nextTick()
}
async function mount() {
  const root = document.createElement('div')
  const app = createApp(UserUsageStats)
  app.mount(root)
  unmount = () => app.unmount()
  await settle()
  return root
}
async function select(root: HTMLElement, index: number, value: string) {
  const control = root.querySelectorAll('select')[index]
  control.value = value
  control.dispatchEvent(new Event('change'))
  await nextTick()
  await vi.advanceTimersByTimeAsync(120)
  await settle()
}
beforeEach(() => {
  vi.useFakeTimers()
  vi.clearAllMocks()
  setI18nLocale('en-US')
  api.users.mockResolvedValue([{ id: 'user-1', username: 'Alice', is_active: true, groups: [{ id: 'group-1' }] }, { id: 'user-2', username: 'Bob', is_active: true, groups: [] }])
  api.groups.mockResolvedValue({ items: [{ id: 'group-1', name: 'Engineering' }, { id: 'group-2', name: 'Support' }] })
  api.members.mockResolvedValue([{ id: 'user-1', is_active: true, is_deleted: false }])
  api.userLeaderboard.mockResolvedValue({ items: [], total: 0 })
  api.groupLeaderboard.mockResolvedValue({ items: [], total: 0 })
  api.summary.mockResolvedValue({ total_requests: 5, total_tokens: 100, total_cost: 2 })
  api.series.mockResolvedValue([])
})
afterEach(() => { unmount(); vi.useRealTimers(); setI18nLocale('zh-CN') })

describe('user and group usage statistics', () => {
  it('applies group scope to summary, trends and member rankings, and can compare groups', async () => {
    const root = await mount()
    expect(api.summary).toHaveBeenLastCalledWith(expect.objectContaining({ user_id: 'user-1' }))
    await select(root, 0, 'user_group')
    expect(api.groupLeaderboard).toHaveBeenCalled()
    expect(api.summary).toHaveBeenLastCalledWith(expect.objectContaining({ user_group_id: 'group-1' }))
    expect(api.summary.mock.lastCall?.[0]).not.toHaveProperty('user_id')
    expect(api.userLeaderboard).toHaveBeenLastCalledWith(expect.objectContaining({ user_group_id: 'group-1', limit: 10 }))
    expect(api.members).toHaveBeenLastCalledWith('group-1')
    await select(root, 2, 'group-2')
    expect(api.series).toHaveBeenCalledWith(expect.objectContaining({ user_group_id: 'group-2' }))
    expect(root.textContent).toContain('Group member leaderboard')
  })

  it('keeps the ungrouped sentinel across scoped queries without fetching a fictitious group', async () => {
    const root = await mount()
    await select(root, 0, 'user_group')
    api.members.mockClear()
    await select(root, 1, '__ungrouped__')
    expect(api.summary).toHaveBeenLastCalledWith(expect.objectContaining({ user_group_id: '__ungrouped__' }))
    expect(api.series).toHaveBeenLastCalledWith(expect.objectContaining({ user_group_id: '__ungrouped__' }))
    expect(api.userLeaderboard).toHaveBeenLastCalledWith(expect.objectContaining({ user_group_id: '__ungrouped__' }))
    expect(api.members).not.toHaveBeenCalled()
  })
})
