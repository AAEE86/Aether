import type { Ref } from 'vue'
import type { EndpointAPIKey } from '@/api/endpoints'
import type { QuotaResetCreditsSnapshot } from '@/api/endpoints/types'
import { getCodexResetCountdown } from '@/composables/useCountdownTimer'
import { useI18n } from '@/i18n'
import type { MessageKey } from '@/i18n/messages'
import { resolveAntigravityQuotaGroupLabel } from '@/features/providers/utils/antigravityQuota'
import {
  getCodexQuotaDisplay,
  getQuotaSnapshotForProvider,
  getQuotaSnapshotUpdatedAt,
  getQuotaWindowByScope,
  getQuotaWindowLiveResetSeconds,
  getQuotaWindowRemainingPercent,
  getQuotaWindowUsedPercent,
} from '@/features/providers/utils/providerKeyQuotaDisplay'
import { getGeminiCliAccountCreditsText } from '@/features/providers/utils/providerKeyQuota'
import {
  getCodexResetCreditAvailableCount as getCodexResetCreditAvailableCountFromSnapshot,
  getVisibleCodexResetCreditItems as getVisibleCodexResetCreditItemsFromSnapshot,
} from '@/features/providers/components/codex-reset-credit-display'

// Codex 剩余额度样式（基于已用百分比计算剩余）
export function getQuotaRemainingClass(usedPercent: number): string {
  const remaining = 100 - usedPercent
  if (remaining <= 10) return 'text-red-600 dark:text-red-400'
  if (remaining <= 30) return 'text-yellow-600 dark:text-yellow-400'
  return 'text-green-600 dark:text-green-400'
}

// Codex 剩余额度进度条颜色
export function getQuotaRemainingBarColor(usedPercent: number): string {
  const remaining = 100 - usedPercent
  if (remaining <= 10) return 'bg-red-500 dark:bg-red-400'
  if (remaining <= 30) return 'bg-yellow-500 dark:bg-yellow-400'
  return 'bg-green-500 dark:bg-green-400'
}

// 判断是否为 Codex Team/Plus/Enterprise 账号（有 5H 限额，显示 3 列）
export function isCodexTeamPlan(key: EndpointAPIKey): boolean {
  const planType = key.oauth_plan_type?.toLowerCase() || getCodexQuotaDisplay(key)?.plan_type?.toLowerCase()
  // Free 账号返回 false（2 列），其他所有账号返回 true（3 列）
  return planType !== undefined && planType !== 'free'
}

export interface AntigravityQuotaItem {
  model: string
  label: string
  usedPercent: number
  remainingPercent: number
  resetSeconds: number | null
  detail?: string
}

export interface ClaudeCodeQuotaItem {
  code: string
  label: string
  usedPercent: number
  remainingPercent: number
  resetSeconds: number | null
}

export const CLAUDE_CODE_QUOTA_WINDOWS: Record<string, { labelKey: MessageKey, sortOrder: number }> = {
  '5h': { labelKey: 'poolQuota.claudeCode.window5h', sortOrder: 0 },
  weekly: { labelKey: 'poolQuota.claudeCode.weekly', sortOrder: 1 },
  weekly_sonnet: { labelKey: 'poolQuota.claudeCode.weeklySonnet', sortOrder: 2 },
  weekly_fable: { labelKey: 'poolQuota.claudeCode.weeklyFable', sortOrder: 3 },
}

export function getClaudeCodeQuotaUpdatedAt(key: EndpointAPIKey): number | undefined {
  const quota = getQuotaSnapshotForProvider(key, 'claude_code')
  const quotaUpdatedAt = getQuotaSnapshotUpdatedAt(quota)
  if (typeof quotaUpdatedAt === 'number') return quotaUpdatedAt
  const updatedAt = Number(key.upstream_metadata?.claude_code?.updated_at ?? NaN)
  return Number.isFinite(updatedAt) && updatedAt > 0 ? updatedAt : undefined
}

export function getClaudeCodeResetCreditsDisplay(key: EndpointAPIKey): QuotaResetCreditsSnapshot | null {
  return getQuotaSnapshotForProvider(key, 'claude_code')?.reset_credits
    ?? key.upstream_metadata?.claude_code?.reset_credits
    ?? null
}

export function getClaudeCodeResetCreditAvailableCount(key: EndpointAPIKey): number | null {
  return getCodexResetCreditAvailableCountFromSnapshot(getClaudeCodeResetCreditsDisplay(key))
}

export interface GeminiCliQuotaItem {
  model: string
  label: string
  usedPercent: number
  remainingPercent: number
  resetSeconds: number | null
}

export function getGeminiCliQuotaUpdatedAt(key: EndpointAPIKey): number | undefined {
  const quota = getQuotaSnapshotForProvider(key, 'gemini_cli')
  const quotaUpdatedAt = getQuotaSnapshotUpdatedAt(quota)
  if (typeof quotaUpdatedAt === 'number') return quotaUpdatedAt
  const updatedAt = Number(key.upstream_metadata?.gemini_cli?.updated_at ?? NaN)
  return Number.isFinite(updatedAt) && updatedAt > 0 ? updatedAt : undefined
}

export function hasGeminiCliQuotaDisplayData(key: EndpointAPIKey): boolean {
  if (getGeminiCliAccountCreditsText(key, 'gemini_cli')) return true
  return getGeminiCliQuotaItems(key).length > 0
}

export function getGeminiCliQuotaItems(key: EndpointAPIKey): GeminiCliQuotaItem[] {
  const quota = getQuotaSnapshotForProvider(key, 'gemini_cli')
  const windows = getQuotaWindowByScope(quota, 'model')
  if (!quota || windows.length === 0) return []

  const items = windows
    .map((window) => {
      const model = String(window.model || window.label || window.code || '').trim()
      if (!model) return null

      const usedPercent = getQuotaWindowUsedPercent(window)
      const remainingPercent = getQuotaWindowRemainingPercent(window)
      if (usedPercent === undefined && remainingPercent === undefined) {
        return null
      }

      const normalizedUsedPercent =
        usedPercent !== undefined
          ? usedPercent
          : Math.max(100 - (remainingPercent ?? 0), 0)
      const normalizedRemainingPercent =
        remainingPercent !== undefined
          ? remainingPercent
          : Math.max(100 - normalizedUsedPercent, 0)

      return {
        model,
        label: String(window.label || window.model || model),
        usedPercent: normalizedUsedPercent,
        remainingPercent: normalizedRemainingPercent,
        resetSeconds: getQuotaWindowLiveResetSeconds(quota, window),
      } satisfies GeminiCliQuotaItem
    })
    .filter((item): item is GeminiCliQuotaItem => item !== null)

  items.sort((a, b) => (b.usedPercent - a.usedPercent) || a.model.localeCompare(b.model))
  return items
}

export function toCodexRemainingPercent(usedPercent: number | null | undefined): number | null {
  const normalizedUsed = Number(usedPercent)
  if (!Number.isFinite(normalizedUsed)) return null
  const clampedUsed = Math.min(Math.max(normalizedUsed, 0), 100)
  return Math.max(100 - clampedUsed, 0)
}

export function shouldStartCodexResetCountdown(usedPercent: number | null | undefined): boolean {
  const remainingPercent = toCodexRemainingPercent(usedPercent)
  if (remainingPercent == null) return true
  return remainingPercent < 100
}

/**
 * 密钥配额展示的本地化与倒计时逻辑。
 * 依赖当前语言与倒计时 tick，因此以 composable 形式提供给抽屉和配额摘要组件共用。
 */
export function useProviderKeyQuotaPresentation(countdownTick: Ref<number>) {
  const { legacyT, locale, t } = useI18n()

  function getWindsurfQuotaStatusLabel(key: EndpointAPIKey): string {
    const quota = getQuotaSnapshotForProvider(key, 'windsurf')
    const label = quota?.label?.trim()
    if (label) return label
    const code = String(quota?.code || '').trim().toLowerCase()
    if (code === 'cooldown') return legacyT('冷却中')
    return code === 'rate_limited' || code === 'rate_limit' ? legacyT('速率受限') : legacyT('额度耗尽')
  }

  // 格式化 Kiro 重置时间
  function formatKiroResetTime(timestamp: number | null | undefined): string {
    if (!timestamp) return ''
    // timestamp 可能是毫秒或秒，需要判断
    const ts = timestamp > 1e12 ? timestamp : timestamp * 1000
    const now = Date.now()
    const diff = ts - now

    if (diff <= 0) {
      return legacyT('已重置')
    }

    const days = Math.floor(diff / (1000 * 60 * 60 * 24))
    const hours = Math.floor((diff % (1000 * 60 * 60 * 24)) / (1000 * 60 * 60))

    if (days > 0) {
      return locale.value === 'en-US' ? `${days}d ${hours}h later` : `${days}天${hours}小时后`
    }

    const minutes = Math.floor((diff % (1000 * 60 * 60)) / (1000 * 60))
    if (hours > 0) {
      return locale.value === 'en-US' ? `${hours}h ${minutes}m later` : `${hours}小时${minutes}分钟后`
    }

    return locale.value === 'en-US' ? `${minutes}m later` : `${minutes}分钟后`
  }

  function getVisibleClaudeCodeResetCreditItems(key: EndpointAPIKey) {
    return getVisibleCodexResetCreditItemsFromSnapshot(
      getClaudeCodeResetCreditsDisplay(key),
      undefined,
      5,
      legacyT('Claude 重置机会'),
    )
  }

  function hasClaudeCodeQuotaDisplayData(key: EndpointAPIKey): boolean {
    return getClaudeCodeQuotaItems(key).length > 0
  }

  function getClaudeCodeQuotaItems(key: EndpointAPIKey): ClaudeCodeQuotaItem[] {
    const quota = getQuotaSnapshotForProvider(key, 'claude_code')
    const windows = quota?.windows
    if (!quota || !Array.isArray(windows)) return []

    return windows
      .map((window) => {
        const code = String(window?.code || '').trim().toLowerCase()
        if (!code) return null

        const usedPercent = getQuotaWindowUsedPercent(window)
        const remainingPercent = getQuotaWindowRemainingPercent(window)
        if (usedPercent === undefined && remainingPercent === undefined) return null

        const normalizedUsedPercent = usedPercent !== undefined
          ? usedPercent
          : Math.max(100 - (remainingPercent ?? 0), 0)
        const normalizedRemainingPercent = remainingPercent !== undefined
          ? remainingPercent
          : Math.max(100 - normalizedUsedPercent, 0)
        const presentation = CLAUDE_CODE_QUOTA_WINDOWS[code]

        return {
          item: {
            code,
            label: t(presentation?.labelKey ?? 'poolQuota.claudeCode.unknownWindow'),
            usedPercent: normalizedUsedPercent,
            remainingPercent: normalizedRemainingPercent,
            resetSeconds: getQuotaWindowLiveResetSeconds(quota, window),
          } satisfies ClaudeCodeQuotaItem,
          sortOrder: presentation?.sortOrder ?? 9,
        }
      })
      .filter((entry): entry is { item: ClaudeCodeQuotaItem, sortOrder: number } => entry !== null)
      .sort((a, b) => a.sortOrder - b.sortOrder)
      .map(entry => entry.item)
  }

  function formatUpdatedAt(updatedAt: number): string {
    if (!updatedAt || typeof updatedAt !== 'number') return ''
    const now = Math.floor(Date.now() / 1000)
    const diff = now - updatedAt
    if (diff <= 60) return legacyT('刚刚更新')
    const minutes = Math.floor(diff / 60)
    if (minutes < 60) return locale.value === 'en-US' ? `${minutes}m ago` : `${minutes}分钟前更新`
    const hours = Math.floor(minutes / 60)
    if (hours < 24) return locale.value === 'en-US' ? `${hours}h ago` : `${hours}小时前更新`
    const days = Math.floor(hours / 24)
    return locale.value === 'en-US' ? `${days}d ago` : `${days}天前更新`
  }

  function getAntigravityQuotaGroupItems(key: EndpointAPIKey): AntigravityQuotaItem[] {
    const quota = getQuotaSnapshotForProvider(key, 'antigravity')
    const windows = getQuotaWindowByScope(quota, 'quota_group')
    if (!quota || windows.length === 0) return []

    return windows
      .map((window) => {
        const model = String(window.code || window.bucket_id || window.label || '').trim()
        if (!model) return null

        const usedPercent = getQuotaWindowUsedPercent(window)
        const remainingPercent = getQuotaWindowRemainingPercent(window)
        if (usedPercent === undefined && remainingPercent === undefined) {
          return null
        }

        const normalizedUsedPercent =
          usedPercent !== undefined
            ? usedPercent
            : Math.max(100 - (remainingPercent ?? 0), 0)
        const normalizedRemainingPercent =
          remainingPercent !== undefined
            ? remainingPercent
            : Math.max(100 - normalizedUsedPercent, 0)

        return {
          model,
          label: resolveAntigravityQuotaGroupLabel(window, t),
          usedPercent: normalizedUsedPercent,
          remainingPercent: normalizedRemainingPercent,
          resetSeconds: getQuotaWindowLiveResetSeconds(quota, window),
        } satisfies AntigravityQuotaItem
      })
      .filter((item): item is AntigravityQuotaItem => item !== null)
  }

  function getResetCountdownText(
    resetAt: number | null | undefined,
    resetSecs: number | null | undefined,
    updatedAt: number | null | undefined,
    usedPercent: number | null | undefined
  ): string {
    const status = getCodexResetCountdown(
      resetAt,
      resetSecs,
      updatedAt,
      countdownTick.value,
      toCodexRemainingPercent(usedPercent)
    )
    if (!status) return ''
    return status.isExpired ? legacyT(status.text) : `${legacyT(status.text)} ${legacyT('后重置')}`
  }

  function getCodexQuotaResetText(
    resetAt: number | null | undefined,
    resetSecs: number | null | undefined,
    updatedAt: number | null | undefined,
    usedPercent: number | null | undefined
  ): string {
    if (resetAt || resetSecs) {
      return getResetCountdownText(resetAt, resetSecs, updatedAt, usedPercent)
    }
    return legacyT('已重置')
  }

  function getResetCountdownClass(
    resetAt: number | null | undefined,
    resetSecs: number | null | undefined,
    updatedAt: number | null | undefined,
    usedPercent: number | null | undefined
  ): string {
    const status = getCodexResetCountdown(
      resetAt,
      resetSecs,
      updatedAt,
      countdownTick.value,
      toCodexRemainingPercent(usedPercent)
    )
    if (!status || status.isExpired) return 'text-muted-foreground/70'
    if (status.isCritical) return 'text-destructive font-medium animate-pulse'
    if (status.isUrgent) return 'text-amber-500 dark:text-amber-400'
    return 'text-muted-foreground/70'
  }

  // 格式化重置时间
  function formatResetTime(seconds: number): string {
    const days = Math.floor(seconds / 86400)
    const hours = Math.floor((seconds % 86400) / 3600)
    const minutes = Math.floor((seconds % 3600) / 60)

    if (days > 0) {
      return locale.value === 'en-US' ? `${days}d ${hours}h` : `${days}天 ${hours}小时`
    }
    if (hours > 0) {
      return locale.value === 'en-US' ? `${hours}h ${minutes}m` : `${hours}小时 ${minutes}分钟`
    }
    return locale.value === 'en-US' ? `${minutes}m` : `${minutes}分钟`
  }

  function hasAntigravityQuotaDisplayData(key: EndpointAPIKey): boolean {
    return getAntigravityQuotaGroupItems(key).length > 0
  }

  return {
    formatUpdatedAt,
    formatKiroResetTime,
    formatResetTime,
    getResetCountdownText,
    getCodexQuotaResetText,
    getResetCountdownClass,
    getWindsurfQuotaStatusLabel,
    hasAntigravityQuotaDisplayData,
    getAntigravityQuotaGroupItems,
    hasClaudeCodeQuotaDisplayData,
    getClaudeCodeQuotaItems,
    getVisibleClaudeCodeResetCreditItems,
  }
}
