import client from '../client'
import type {
  OAuth2ProviderInfo,
  OAuth2AuthorizeResponse,
  OAuth2CallbackResponse,
} from './types'

// Re-export types for convenience
export type { OAuth2ProviderInfo, OAuth2AuthorizeResponse, OAuth2CallbackResponse }

/**
 * 获取所有 OAuth2 Provider 信息
 */
export async function getOAuth2Providers(): Promise<OAuth2ProviderInfo[]> {
  const response = await client.get('/api/admin/oauth2/providers')
  return response.data.providers
}

/**
 * 开始 OAuth2 授权流程
 */
export async function startOAuth2Authorize(
  providerId: string,
): Promise<OAuth2AuthorizeResponse> {
  const response = await client.post(`/api/admin/oauth2/authorize/${providerId}`)
  return response.data
}

/**
 * 轮询获取 OAuth2 授权结果
 */
export async function pollOAuth2Result(state: string): Promise<OAuth2CallbackResponse> {
  const response = await client.post('/api/admin/oauth2/result', { state })
  return response.data
}

/**
 * 提交手动复制的回调 URL（用于 manual 模式）
 */
export async function submitManualCallback(
  callbackUrl: string,
  state: string
): Promise<OAuth2CallbackResponse> {
  const response = await client.post('/api/admin/oauth2/manual-callback', {
    callback_url: callbackUrl,
    state,
  })
  return response.data
}

/**
 * OAuth2 授权流程辅助类（自动模式）
 */
export class OAuth2AuthFlow {
  private providerId: string
  private pollInterval: number
  private maxPollTime: number
  private authWindow: Window | null = null
  private pollTimer: ReturnType<typeof setInterval> | null = null

  constructor(
    providerId: string,
    options?: {
      pollInterval?: number  // 轮询间隔（毫秒），默认 2000
      maxPollTime?: number   // 最大轮询时间（毫秒），默认 300000（5分钟）
    }
  ) {
    this.providerId = providerId
    this.pollInterval = options?.pollInterval || 2000
    this.maxPollTime = options?.maxPollTime || 300000
  }

  /**
   * 开始授权流程
   * @returns Promise 包含 token_data 或错误
   */
  async start(): Promise<OAuth2CallbackResponse> {
    // 1. 获取授权 URL
    const authResponse = await startOAuth2Authorize(this.providerId)

    // 2. 打开授权窗口
    const width = 600
    const height = 700
    const left = window.screenX + (window.outerWidth - width) / 2
    const top = window.screenY + (window.outerHeight - height) / 2

    this.authWindow = window.open(
      authResponse.authorization_url,
      `oauth2_${this.providerId}`,
      `width=${width},height=${height},left=${left},top=${top},scrollbars=yes,resizable=yes`
    )

    if (!this.authWindow) {
      throw new Error('无法打开授权窗口，请检查浏览器弹窗设置')
    }

    // 3. 轮询获取结果
    return new Promise<OAuth2CallbackResponse>((resolve, reject) => {
      const startTime = Date.now()

      this.pollTimer = setInterval(async () => {
        // 检查窗口是否被关闭
        if (this.authWindow?.closed) {
          this.cleanup()
          reject(new Error('授权窗口已关闭'))
          return
        }

        // 检查是否超时
        if (Date.now() - startTime > this.maxPollTime) {
          this.cleanup()
          reject(new Error('授权超时，请重试'))
          return
        }

        try {
          const result = await pollOAuth2Result(authResponse.state)
          if (result.success || result.error) {
            this.cleanup()
            resolve(result)
          }
          // 如果没有结果，继续轮询
        } catch {
          // 忽略轮询错误，继续尝试
        }
      }, this.pollInterval)
    })
  }

  /**
   * 清理资源
   */
  cleanup() {
    if (this.pollTimer) {
      clearInterval(this.pollTimer)
      this.pollTimer = null
    }
    if (this.authWindow && !this.authWindow.closed) {
      this.authWindow.close()
    }
    this.authWindow = null
  }

  /**
   * 取消授权流程
   */
  cancel() {
    this.cleanup()
  }
}

/**
 * OAuth2 手动授权流程辅助类（manual 模式）
 *
 * 用于 ClaudeCode、Codex 等需要手动复制回调 URL 的 Provider。
 * 流程：
 * 1. 打开授权窗口
 * 2. 用户在浏览器中完成授权
 * 3. 用户复制授权完成后的 URL
 * 4. 用户粘贴到前端输入框
 * 5. 前端调用 submitManualCallback 提交 URL
 */
export class OAuth2ManualAuthFlow {
  private providerId: string
  private authWindow: Window | null = null
  private _authResponse: OAuth2AuthorizeResponse | null = null

  constructor(providerId: string) {
    this.providerId = providerId
  }

  /**
   * 获取授权响应（包含 state 等信息）
   */
  get authResponse() {
    return this._authResponse
  }

  /**
   * 开始授权流程 - 打开授权窗口
   * @returns 授权响应，包含 state 等信息
   */
  async start(): Promise<OAuth2AuthorizeResponse> {
    // 1. 获取授权 URL
    this._authResponse = await startOAuth2Authorize(this.providerId)

    // 2. 打开授权窗口
    const width = 600
    const height = 700
    const left = window.screenX + (window.outerWidth - width) / 2
    const top = window.screenY + (window.outerHeight - height) / 2

    this.authWindow = window.open(
      this._authResponse.authorization_url,
      `oauth2_${this.providerId}`,
      `width=${width},height=${height},left=${left},top=${top},scrollbars=yes,resizable=yes`
    )

    if (!this.authWindow) {
      throw new Error('无法打开授权窗口，请检查浏览器弹窗设置')
    }

    return this._authResponse
  }

  /**
   * 提交用户粘贴的回调 URL
   * @param callbackUrl 用户复制的完整回调 URL
   * @returns 授权结果
   */
  async submitCallbackUrl(callbackUrl: string): Promise<OAuth2CallbackResponse> {
    if (!this._authResponse) {
      throw new Error('请先调用 start() 开始授权流程')
    }

    const result = await submitManualCallback(callbackUrl, this._authResponse.state)
    this.cleanup()
    return result
  }

  /**
   * 清理资源
   */
  cleanup() {
    if (this.authWindow && !this.authWindow.closed) {
      this.authWindow.close()
    }
    this.authWindow = null
  }

  /**
   * 取消授权流程
   */
  cancel() {
    this.cleanup()
    this._authResponse = null
  }
}
