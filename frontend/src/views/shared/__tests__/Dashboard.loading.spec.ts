import { createApp, nextTick, type App } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { OverviewDashboardSummary } from '@/api/overview'
import { dashboardSummary } from '@/features/overview/__tests__/fixtures/dashboardSummary'
import Dashboard from '../Dashboard.vue'

const api = vi.hoisted(() => ({ dashboardSummary: vi.fn(), summary: vi.fn(), dashboardTotal: vi.fn(), daily: vi.fn() }))
vi.mock('@/stores/auth', () => ({ useAuthStore: () => ({ canAccessAdmin: true, isAdmin: true, isAuditAdmin: false }) }))
vi.mock('@/api/overview', () => ({ overviewApi: api }))
vi.mock('@/api/dashboard', () => ({ dashboardApi: { getDailyStats: api.daily } }))
vi.mock('@/features/overview/dashboard/DashboardActivity.vue', () => ({ default: { render: () => null } }))
vi.mock('@/features/overview/dashboard/DashboardAnnouncements.vue', () => ({ default: { render: () => null } }))
vi.mock('@/components/charts/BarChart.vue', () => ({ default: { render: () => null } }))
vi.mock('@/components/charts/DoughnutChart.vue', () => ({ default: { render: () => null } }))
vi.mock('@/components/charts/LineChart.vue', () => ({ default: { render: () => null } }))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: Error) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => { resolve = resolvePromise; reject = rejectPromise })
  return { promise, resolve, reject }
}
let app: App | undefined
function mount() {
  const root = document.createElement('div')
  app = createApp(Dashboard)
  app.mount(root)
  return root
}
async function settle() {
  for (let i = 0; i < 8; i += 1) { await Promise.resolve(); await nextTick() }
}
beforeEach(() => {
  vi.useFakeTimers()
  vi.resetAllMocks()
  api.dashboardSummary.mockResolvedValue(dashboardSummary())
  api.daily.mockResolvedValue({ daily_stats: [], provider_summary: [] })
})
afterEach(() => { app?.unmount(); app = undefined; vi.useRealTimers() })

describe('dashboard snapshot loading', () => {
  it('shows today and total from one compact snapshot independently of pending charts', async () => {
    api.daily.mockReturnValue(new Promise(() => {}))
    const root = mount()
    await settle()
    expect(root.textContent).toContain('总请求 12,345')
    expect(root.textContent).toContain('$9.87')
    expect(root.textContent).toContain('1.52s')
    expect(root.querySelector('[aria-busy="true"]')).toBeNull()
    expect(api.dashboardSummary).toHaveBeenCalledWith(Intl.DateTimeFormat().resolvedOptions().timeZone, expect.any(AbortSignal))
    expect(api.summary).not.toHaveBeenCalled()
    expect(api.dashboardTotal).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(240_000)
    expect(api.dashboardSummary).toHaveBeenCalledTimes(1)
    expect(api.daily).toHaveBeenCalledTimes(1)
  })

  it('does not loop requests with the real time picker when summary fails', async () => {
    const snapshot = deferred<OverviewDashboardSummary>()
    api.dashboardSummary.mockReturnValue(snapshot.promise)
    const root = mount()
    await settle()
    expect(root.querySelector('[aria-busy="true"]')).not.toBeNull()
    snapshot.reject(new Error('timeout'))
    await settle()
    expect(root.querySelector('[aria-busy="true"]')).toBeNull()
    expect(root.querySelector('[role="alert"]')).toBeNull()
    expect(root.textContent).not.toContain('加载失败')
    expect(root.textContent).not.toContain('重试')
    expect(root.querySelector('[data-request-metric="stream"]')?.textContent).toContain('—')
    await vi.advanceTimersByTimeAsync(61_000)
    expect(api.dashboardSummary).toHaveBeenCalledTimes(1)
    expect(api.daily.mock.calls.length).toBeLessThanOrEqual(2)
  })

  it('aborts the compact snapshot request when unmounted and ignores late success', async () => {
    const snapshot = deferred<OverviewDashboardSummary>()
    api.dashboardSummary.mockReturnValue(snapshot.promise)
    const root = mount()
    await settle()
    const signal = api.dashboardSummary.mock.calls[0]![1] as AbortSignal
    app?.unmount()
    app = undefined
    expect(signal.aborted).toBe(true)
    snapshot.resolve(dashboardSummary())
    await settle()
    expect(root.textContent).toBe('')
    await vi.advanceTimersByTimeAsync(240_000)
    expect(api.dashboardSummary).toHaveBeenCalledTimes(1)
  })
})
