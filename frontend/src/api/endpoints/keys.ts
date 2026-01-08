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
 * API Key 基础配置类型
 */
export interface APIKeyBaseConfig {
  api_key: string
  name: string
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
  note?: string
}

/**
 * Endpoint Key 创建参数
 */
export interface EndpointKeyCreateData extends APIKeyBaseConfig {
  endpoint_id: string
}

/**
 * Provider Key 创建参数
 */
export type ProviderKeyCreateData = APIKeyBaseConfig

/**
 * API Key 更新参数（所有字段可选，支持 null 清空某些字段）
 */
export interface APIKeyUpdateData {
  api_key?: string
  name?: string
  rate_multiplier?: number
  internal_priority?: number
  global_priority?: number
  max_concurrent?: number | null  // null 表示切换为自适应模式
  rate_limit?: number
  daily_limit?: number
  monthly_limit?: number
  cache_ttl_minutes?: number
  max_probe_interval_minutes?: number
  allowed_models?: string[] | null  // null 表示允许所有模型
  capabilities?: Record<string, boolean> | null  // null 表示清空能力配置
  note?: string
  is_active?: boolean
}

/**
 * 批量优先级更新参数
 */
export interface BatchPriorityUpdate {
  key_id: string
  internal_priority: number
}

/**
 * 批量优先级更新响应
 */
export interface BatchPriorityUpdateResponse {
  message: string
  updated_count: number
}

/**
 * Key 查看响应
 */
export interface KeyRevealResponse {
  api_key: string
}

/**
 * 删除响应
 */
export interface DeleteResponse {
  message: string
}

// ==================== 能力相关 API ====================

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

// ==================== Endpoint Keys API ====================

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
  data: EndpointKeyCreateData
): Promise<EndpointAPIKey> {
  const response = await client.post(`/api/admin/endpoints/${endpointId}/keys`, data)
  return response.data
}

/**
 * 更新 Endpoint Key
 */
export async function updateEndpointKey(
  keyId: string,
  data: APIKeyUpdateData
): Promise<EndpointAPIKey> {
  const response = await client.put(`/api/admin/endpoints/keys/${keyId}`, data)
  return response.data
}

/**
 * 获取完整的 API Key（用于查看和复制）
 */
export async function revealEndpointKey(keyId: string): Promise<KeyRevealResponse> {
  const response = await client.get(`/api/admin/endpoints/keys/${keyId}/reveal`)
  return response.data
}

/**
 * 删除 Endpoint Key
 */
export async function deleteEndpointKey(keyId: string): Promise<DeleteResponse> {
  const response = await client.delete(`/api/admin/endpoints/keys/${keyId}`)
  return response.data
}

/**
 * 批量更新 Endpoint Keys 的优先级（用于拖动排序）
 */
export async function batchUpdateKeyPriority(
  endpointId: string,
  priorities: BatchPriorityUpdate[]
): Promise<BatchPriorityUpdateResponse> {
  const response = await client.put(`/api/admin/endpoints/${endpointId}/keys/batch-priority`, {
    priorities
  })
  return response.data
}

// ==================== Provider Shared Keys API ====================

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
  data: ProviderKeyCreateData
): Promise<EndpointAPIKey> {
  const response = await client.post(`/api/admin/providers/${providerId}/keys`, data)
  return response.data
}

/**
 * 更新 Provider Shared Key
 */
export async function updateProviderKey(
  keyId: string,
  data: APIKeyUpdateData
): Promise<EndpointAPIKey> {
  const response = await client.put(`/api/admin/providers/keys/${keyId}`, data)
  return response.data
}

/**
 * 删除 Provider Shared Key
 */
export async function deleteProviderKey(keyId: string): Promise<DeleteResponse> {
  const response = await client.delete(`/api/admin/providers/keys/${keyId}`)
  return response.data
}

/**
 * 获取完整的 Provider Shared Key
 */
export async function revealProviderKey(keyId: string): Promise<KeyRevealResponse> {
  const response = await client.get(`/api/admin/providers/keys/${keyId}/reveal`)
  return response.data
}

/**
 * 批量更新 Provider Shared Keys 的优先级（用于拖动排序）
 */
export async function batchUpdateProviderKeyPriority(
  providerId: string,
  priorities: BatchPriorityUpdate[]
): Promise<BatchPriorityUpdateResponse> {
  const response = await client.put(`/api/admin/providers/${providerId}/keys/batch-priority`, {
    priorities
  })
  return response.data
}
