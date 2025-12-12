import apiClient from './client'
import { log } from '@/utils/logger'

export interface LoginRequest {
  email: string
  password: string
}

export interface LoginResponse {
  access_token: string
  refresh_token?: string
  token_type?: string
  expires_in?: number
  user_id?: string // UUID
  email?: string
  username?: string
  role?: string
}

export interface UserPreferences {
  theme?: 'light' | 'dark' | 'auto'
  language?: string
  notifications_enabled?: boolean
  [key: string]: unknown // 允许扩展其他偏好设置
}

export interface UserStats {
  total_requests?: number
  total_cost?: number
  last_request_at?: string
  [key: string]: unknown // 允许扩展其他统计数据
}

export interface User {
  id: string // UUID
  username: string
  email?: string
  role: string  // 'admin' or 'user'
  is_active: boolean
  quota_usd?: number | null
  used_usd?: number
  total_usd?: number
  allowed_providers?: string[] | null  // 允许使用的提供商 ID 列表
  allowed_endpoints?: string[] | null  // 允许使用的端点 ID 列表
  allowed_models?: string[] | null  // 允许使用的模型名称列表
  created_at: string
  last_login_at?: string
  preferences?: UserPreferences
  stats?: UserStats
}

export const authApi = {
  async login(credentials: LoginRequest): Promise<LoginResponse> {
    const response = await apiClient.post<LoginResponse>('/api/auth/login', credentials)
    apiClient.setToken(response.data.access_token)
    // 后端暂时没有返回 refresh_token
    if (response.data.refresh_token) {
      localStorage.setItem('refresh_token', response.data.refresh_token)
    }
    return response.data
  },

  async logout() {
    try {
      // 调用后端登出接口，将 Token 加入黑名单
      await apiClient.post('/api/auth/logout', {})
    } catch (error) {
      // 即使后端登出失败，也要清除本地认证信息
      log.warn('后端登出失败，仅清除本地认证信息:', error)
    } finally {
      // 清除本地认证信息
      apiClient.clearAuth()
    }
  },

  async getCurrentUser(): Promise<User> {
    const response = await apiClient.get<User>('/api/users/me')
    return response.data
  },

  async refreshToken(refreshToken: string): Promise<LoginResponse> {
    const response = await apiClient.post<LoginResponse>('/api/auth/refresh', {
      refresh_token: refreshToken
    })
    apiClient.setToken(response.data.access_token)
    if (response.data.refresh_token) {
      localStorage.setItem('refresh_token', response.data.refresh_token)
    }
    return response.data
  }
}
