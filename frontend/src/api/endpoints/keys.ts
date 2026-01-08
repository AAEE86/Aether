import client from '../client'
import type { EndpointAPIKey } from './types'

/**
 * 能力定义类型
 */
export interface CapabilityDefinition {
  name: string
  display_name: string
  description: string
  match_mode: 'exclusive' | 'compatible'
  config_mode?: 'user_configurable' | 'auto_detect' | 'request_param'
  short_name?: string
}

/**
 * 模型支持的能力响应类型
 */
export interface ModelCapabilitiesResponse {
  model: string
  global_model_id?: string
  global_model_name?: string
  supported_capabilities: string[]
  capability_details: CapabilityDefinition[]
  error?: string
}

/**
 * 获取所有能力定义
 */
export async function getAllCapabilities(): Promise<CapabilityDefinition[]> {
  const response = await client.get('/api/capabilities')
  return response.data.capabilities
}

/**
 * 获取用户可配置的能力列表
 */
export async function getUserConfigurableCapabilities(): Promise<CapabilityDefinition[]> {
  const response = await client.get('/api/capabilities/user-configurable')
  return response.data.capabilities
}

/**
 * 获取指定模型支持的能力列表
 */
export async function getModelCapabilities(modelName: string): Promise<ModelCapabilitiesResponse> {
  const response = await client.get(`/api/capabilities/model/${encodeURIComponent(modelName)}`)
  return response.data
}

/**
 * 获取 Endpoint 的所有 Keys
 */
export async function getEndpointKeys(endpointId: string): Promise<EndpointAPIKey[]> {
  const response = await client.get(`/api/admin/endpoints/${endpointId}/keys`)
  return response.data
}

/**
 * 为 Endpoint 添加 Key
 */
export async function addEndpointKey(
  endpointId: string,
  data: {
    endpoint_id: string
    api_key: string
    name: string  // 密钥名称（必填）
    rate_multiplier?: number  // 成本倍率（默认 1.0）
    internal_priority?: number  // Endpoint 内部优先级（数字越小越优先）
    max_concurrent?: number  // 最大并发数（留空=自适应模式）
    rate_limit?: number
    daily_limit?: number
    monthly_limit?: number
    cache_ttl_minutes?: number  // 缓存 TTL（分钟），0=禁用
    max_probe_interval_minutes?: number  // 熔断探测间隔（分钟）
    allowed_models?: string[]  // 允许使用的模型列表
    capabilities?: Record<string, boolean>  // 能力标签配置
    note?: string  // 备注说明（可选）
  }
): Promise<EndpointAPIKey> {
  const response = await client.post(`/api/admin/endpoints/${endpointId}/keys`, data)
  return response.data
}

/**
 * 更新 Endpoint Key
 */
export async function updateEndpointKey(
  keyId: string,
  data: Partial<{
    api_key: string
    name: string  // 密钥名称
    rate_multiplier: number  // 成本倍率
    internal_priority: number  // Endpoint 内部优先级（提供商优先模式，数字越小越优先）
    global_priority: number  // 全局 Key 优先级（全局 Key 优先模式，数字越小越优先）
    max_concurrent: number  // 最大并发数（留空=自适应模式）
    rate_limit: number
    daily_limit: number
    monthly_limit: number
    cache_ttl_minutes: number  // 缓存 TTL（分钟），0=禁用
    max_probe_interval_minutes: number  // 熔断探测间隔（分钟）
    allowed_models: string[] | null  // 允许使用的模型列表，null 表示允许所有
    capabilities: Record<string, boolean> | null  // 能力标签配置
    is_active: boolean
    note: string  // 备注说明
  }>
): Promise<EndpointAPIKey> {
  const response = await client.put(`/api/admin/endpoints/keys/${keyId}`, data)
  return response.data
}

/**
 * 获取完整的 API Key（用于查看和复制）
 */
export async function revealEndpointKey(keyId: string): Promise<{ api_key: string }> {
  const response = await client.get(`/api/admin/endpoints/keys/${keyId}/reveal`)
  return response.data
}

/**
 * 删除 Endpoint Key
 */
export async function deleteEndpointKey(keyId: string): Promise<{ message: string }> {
  const response = await client.delete(`/api/admin/endpoints/keys/${keyId}`)
  return response.data
}

/**
 * 批量更新 Endpoint Keys 的优先级（用于拖动排序）
 */
export async function batchUpdateKeyPriority(
  endpointId: string,
  priorities: Array<{ key_id: string; internal_priority: number }>
): Promise<{ message: string; updated_count: number }> {
  const response = await client.put(`/api/admin/endpoints/${endpointId}/keys/batch-priority`, {
    priorities
  })
  return response.data
}

// ========== Provider Shared Keys ==========

/**
 * 获取 Provider 的所有 Shared Keys
 */
export async function getProviderKeys(providerId: string): Promise<EndpointAPIKey[]> {
  const response = await client.get(`/api/admin/providers/${providerId}/keys`)
  return response.data
}

/**
 * 为 Provider 添加 Shared Key
 */
export async function addProviderKey(
  providerId: string,
  data: {
    api_key: string
    name: string
    note?: string
    rate_multiplier?: number
    internal_priority?: number
    max_concurrent?: number
    rate_limit?: number
    daily_limit?: number
    monthly_limit?: number
    cache_ttl_minutes?: number
    max_probe_interval_minutes?: number
    allowed_models?: string[]
    capabilities?: Record<string, boolean>
  }
): Promise<EndpointAPIKey> {
  const response = await client.post(`/api/admin/providers/${providerId}/keys`, data)
  return response.data
}

/**
 * 更新 Provider Shared Key
 */
export async function updateProviderKey(
  keyId: string,
  data: Partial<{
    api_key: string
    name: string
    rate_multiplier: number
    internal_priority: number
    max_concurrent: number
    rate_limit: number
    daily_limit: number
    monthly_limit: number
    cache_ttl_minutes: number
    max_probe_interval_minutes: number
    allowed_models: string[] | null
    capabilities: Record<string, boolean> | null
    is_active: boolean
    note: string
  }>
): Promise<EndpointAPIKey> {
  const response = await client.put(`/api/admin/providers/keys/${keyId}`, data)
  return response.data
}

/**
 * 删除 Provider Shared Key
 */
export async function deleteProviderKey(keyId: string): Promise<{ message: string }> {
  const response = await client.delete(`/api/admin/providers/keys/${keyId}`)
  return response.data
}

/**
 * 获取完整的 Provider Shared Key
 */
export async function revealProviderKey(keyId: string): Promise<{ api_key: string }> {
  const response = await client.get(`/api/admin/providers/keys/${keyId}/reveal`)
  return response.data
}

/**
 * 批量更新 Provider Shared Keys 的优先级（用于拖动排序）
 */
export async function batchUpdateProviderKeyPriority(
  providerId: string,
  priorities: Array<{ key_id: string; internal_priority: number }>
): Promise<{ message: string; updated_count: number }> {
  const response = await client.put(`/api/admin/providers/${providerId}/keys/batch-priority`, {
    priorities
  })
  return response.data
}
