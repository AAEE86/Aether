import { beforeEach, describe, expect, it, vi } from 'vitest'

const { getAllSystemConfigsMock, updateSystemConfigMock } = vi.hoisted(() => ({
  getAllSystemConfigsMock: vi.fn(),
  updateSystemConfigMock: vi.fn(),
}))

vi.mock('@/api/admin', () => ({
  adminApi: {
    getAllSystemConfigs: getAllSystemConfigsMock,
    updateSystemConfig: updateSystemConfigMock,
    getSystemVersion: vi.fn(),
  },
}))

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({
    success: vi.fn(),
    error: vi.fn(),
  }),
}))

vi.mock('@/composables/useSiteInfo', () => ({
  useSiteInfo: () => ({
    refreshSiteInfo: vi.fn(),
  }),
}))

vi.mock('@/utils/logger', () => ({
  log: {
    error: vi.fn(),
  },
}))

import { useSystemConfig } from '../composables/useSystemConfig'

describe('useSystemConfig', () => {
  beforeEach(() => {
    getAllSystemConfigsMock.mockReset()
    updateSystemConfigMock.mockReset()
  })

  it('loads config keys in one request and keeps change detection disabled until the baseline is ready', async () => {
    let resolveConfigs: ((value: Array<{ key: string, value: unknown, is_set?: boolean }>) => void) | null = null
    getAllSystemConfigsMock.mockImplementation(() => new Promise((resolve) => {
      resolveConfigs = resolve
    }))

    const state = useSystemConfig()
    const loadPromise = state.loadSystemConfig()

    expect(getAllSystemConfigsMock).toHaveBeenCalledTimes(1)
    expect(getAllSystemConfigsMock).toHaveBeenCalledWith({ cacheTtlMs: 30_000 })

    state.systemConfig.value.request_record_level = 'headers'
    expect(state.systemConfigLoading.value).toBe(true)
    expect(state.hasLogConfigChanges.value).toBe(false)

    resolveConfigs?.([
      { key: 'request_record_level', value: 'basic' },
      { key: 'proxy_node_metrics_cleanup_batch_size', value: 5000 },
    ])
    await loadPromise

    expect(state.systemConfigLoading.value).toBe(false)
    expect(state.systemConfig.value.request_record_level).toBe('basic')
    expect(state.hasLogConfigChanges.value).toBe(false)

    state.systemConfig.value.request_record_level = 'full'
    expect(state.hasLogConfigChanges.value).toBe(true)
  })

  it('uses backend-compatible defaults when config rows have not been persisted yet', async () => {
    getAllSystemConfigsMock.mockResolvedValue([])

    const state = useSystemConfig()
    await state.loadSystemConfig()

    expect(state.systemConfig.value.request_record_level).toBe('basic')
    expect(state.systemConfig.value).not.toHaveProperty('max_request_body_size')
    expect(state.systemConfig.value).not.toHaveProperty('max_response_body_size')
  })

  it('loads and saves the extra trusted Fake-IP DNS hosts with proxy settings', async () => {
    getAllSystemConfigsMock.mockResolvedValue([
      { key: 'system_proxy_node_id', value: 'node-1' },
      { key: 'execution_extra_trusted_dns_hosts', value: ['custom.example.com'] },
    ])
    updateSystemConfigMock.mockResolvedValue(undefined)

    const state = useSystemConfig()
    await state.loadSystemConfig()

    expect(state.extraTrustedDnsHostsStr.value).toBe('custom.example.com')
    state.extraTrustedDnsHostsStr.value = 'api.example.com\nmodels.example.com'
    expect(state.systemConfig.value.execution_extra_trusted_dns_hosts).toEqual([
      'api.example.com',
      'models.example.com',
    ])

    await state.saveProxyConfig()

    expect(updateSystemConfigMock).toHaveBeenCalledWith(
      'execution_extra_trusted_dns_hosts',
      ['api.example.com', 'models.example.com'],
      '执行运行时额外可信 Fake-IP 域名'
    )
    expect(state.hasProxyConfigChanges.value).toBe(false)
  })
})
