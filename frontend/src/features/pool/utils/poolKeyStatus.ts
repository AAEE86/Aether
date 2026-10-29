import type { PoolKeyDetail } from '@/api/endpoints/pool'
import { buildPoolMobileTagItems } from '@/features/pool/utils/poolMobilePresentation'
import type { PoolMobileTagItem, PoolMobileTagTone } from '@/features/pool/utils/poolMobilePresentation'
import { getOAuthOrgBadge } from '@/features/providers/utils/oauthIdentity'
import { formatOAuthPlanType } from '@/features/providers/utils/oauthPlanType'
import { getProviderAuthLabel } from '@/features/providers/utils/providerKeyAuth'
import {
  getAccountStatusDisplay,
  getAccountStatusTitle,
  getOAuthRefreshButtonTitle as resolveOAuthRefreshButtonTitle,
  getOAuthStatusDisplayWithFallback,
  getOAuthStatusTitle as resolveOAuthStatusTitle,
} from '@/features/providers/utils/providerKeyStatus'
import { getLegacyAccountQuotaText } from '@/features/providers/utils/providerKeyQuota'
import { getQuotaSnapshot, resolvePoolKeyPlanType } from './poolKeyQuotaProgress'

export type PoolKeyScore = NonNullable<PoolKeyDetail['pool_score']>

export const poolScoreHardStateOptions = [
  { value: 'all', label: '全部状态' },
  { value: 'available', label: '可用' },
  { value: 'unknown', label: '未知' },
  { value: 'cooldown', label: '冷却' },
  { value: 'quota_exhausted', label: '额度耗尽' },
  { value: 'auth_invalid', label: '授权无效' },
  { value: 'banned', label: '封禁' },
  { value: 'inactive', label: '禁用' },
]

export const poolScoreProbeStatusOptions = [
  { value: 'all', label: '全部探测' },
  { value: 'never', label: '未探测' },
  { value: 'ok', label: '正常' },
  { value: 'failed', label: '失败' },
  { value: 'stale', label: '过期' },
  { value: 'in_progress', label: '探测中' },
]

export const COOLDOWN_REASON_MAP: Record<string, string> = {
  rate_limited_429: '429 限流',
  forbidden_403: '403 禁止',
  overloaded_529: '529 过载',
  auth_failed_401: '401 认证失败',
  payment_required_402: '402 欠费',
  server_error_500: '500 错误',
  request_timeout_408: '408 超时',
  conflict_409: '409 冲突',
  locked_423: '423 锁定',
  too_early_425: '425 Too Early',
  bad_gateway_502: '502 网关错误',
  service_unavailable_503: '503 服务不可用',
  gateway_timeout_504: '504 网关超时',
}

export function formatCooldownReason(reason: string): string {
  return COOLDOWN_REASON_MAP[reason] || reason
}

export type PoolStatusVariant = 'default' | 'secondary' | 'destructive' | 'outline' | 'success' | 'warning' | 'dark'

export function isHealthDerivedSchedulingReason(reason: string | null | undefined): boolean {
  const normalized = String(reason || '').trim().toLowerCase()
  return normalized === 'health_low'
    || normalized === 'health_degraded'
    || normalized === 'health'
    || normalized === 'circuit_open'
    || normalized === 'circuit_breaker'
}

export function isHealthDerivedSchedulingLabel(label: string | null | undefined): boolean {
  const normalized = String(label || '').trim()
  return normalized === '健康低'
    || normalized === '健康度较低'
    || normalized === '降级'
    || normalized === '熔断'
    || normalized === '熔断中'
}

export function getVisibleSchedulingReason(key: PoolKeyDetail): string | null {
  const reason = String(key.scheduling_reason || '').trim()
  if (!reason || isHealthDerivedSchedulingReason(reason)) return null
  return reason
}

export function getVisibleSchedulingReasons(key: PoolKeyDetail) {
  return (key.scheduling_reasons ?? []).filter((item) => {
    const source = String(item.source || '').trim().toLowerCase()
    return source !== 'health'
      && !isHealthDerivedSchedulingReason(item.code)
      && !isHealthDerivedSchedulingLabel(item.label)
  })
}

export function getSchedulingStatus(key: PoolKeyDetail, tick: number): 'available' | 'degraded' | 'blocked' {
  if (getAccountAlertLabel(key)) return 'blocked'
  if (getBlockingOAuthStatusLabel(key, tick)) return 'blocked'

  const status = key.scheduling_status
  if (
    (status === 'available' || status === 'degraded' || status === 'blocked')
    && !isHealthDerivedSchedulingReason(key.scheduling_reason)
    && !isHealthDerivedSchedulingLabel(key.scheduling_label)
  ) {
    return status
  }

  if (!key.is_active) return 'blocked'
  if (key.cooldown_reason) return 'degraded'
  if (key.cost_limit != null && key.cost_limit > 0 && key.cost_window_usage >= key.cost_limit) return 'blocked'
  return 'available'
}

export function compactPoolStatusLabel(label: string | null | undefined): string | null {
  const normalized = String(label || '').trim()
  if (!normalized) return null

  const mapped: Record<string, string> = {
    'Token 失效': '已失效',
    'Token 过期': '已过期',
    Token失效: '已失效',
    Token过期: '已过期',
    账号已封禁: '账号封禁',
    工作区已停用: '工作区停用',
    账号访问受限: '访问受限',
    健康度较低: '健康低',
  }
  const labelText = mapped[normalized] || normalized
  return Array.from(labelText).slice(0, 5).join('')
}

export function getOAuthStatusBadgeLabel(status: ReturnType<typeof getVisibleOAuthState>): string | null {
  if (!status) return null
  if (status.requiresReauth) return '续期失败'
  if (status.isInvalid) return '已失效'
  if (status.isExpired) return '已过期'
  if (status.text === '未添加') return '未添加'
  if (status.text === '有效期未知') return '未知'
  if (status.isExpiringSoon) return '将过期'
  return '有效'
}

export function getBlockingOAuthStatusLabel(key: PoolKeyDetail, tick: number): string | null {
  const oauthState = getVisibleOAuthState(key, tick)
  if (!oauthState?.isInvalid && !oauthState?.isExpired) return null
  return getOAuthStatusBadgeLabel(oauthState)
}

export function isPoolKeyCostExhausted(key: PoolKeyDetail): boolean {
  return key.cost_limit != null
    && key.cost_limit > 0
    && key.cost_window_usage >= key.cost_limit
}

export function getSchedulingBadgeLabel(key: PoolKeyDetail, tick: number): string {
  const accountAlert = getAccountAlertLabel(key)
  if (accountAlert) return compactPoolStatusLabel(accountAlert) || accountAlert
  const oauthAlert = getBlockingOAuthStatusLabel(key, tick)
  if (oauthAlert) return oauthAlert

  const rawLabel = String(key.scheduling_label || '').trim()
  if (
    rawLabel
    && !isHealthDerivedSchedulingReason(key.scheduling_reason)
    && !isHealthDerivedSchedulingLabel(rawLabel)
  ) {
    if ((rawLabel === '可用' || key.scheduling_reason === 'available') && isPoolKeyCostExhausted(key)) {
      return '超限'
    }
    if (rawLabel === '禁用' || rawLabel === '停用') return '禁用'
    return compactPoolStatusLabel(rawLabel) || rawLabel
  }

  if (!key.is_active) return '已禁用'
  if (key.cooldown_reason) return '冷却中'
  return '可用'
}

export function getSchedulingBadgeVariant(key: PoolKeyDetail, tick: number): PoolStatusVariant {
  if (getAccountAlertLabel(key)) return 'destructive'
  if (getBlockingOAuthStatusLabel(key, tick)) return 'destructive'

  const reason = getVisibleSchedulingReason(key)
  if (reason === 'manual_disabled' || reason === 'inactive') return 'secondary'
  if (reason === 'account_blocked' || reason === 'account_quota_exhausted' || reason === 'cost_exhausted') return 'destructive'
  if (reason === 'cooldown') return 'warning'
  if (reason === 'cost_soft' || reason === 'cost') return 'warning'
  if (isPoolKeyCostExhausted(key)) return 'destructive'
  if (reason === 'available') return 'default'
  if (!reason && !key.is_active) return 'secondary'

  const status = getSchedulingStatus(key, tick)
  if (status === 'blocked') return 'destructive'
  if (status === 'degraded') return 'warning'
  return 'default'
}

export function getSchedulingTitle(key: PoolKeyDetail, tick: number): string {
  const accountAlertTitle = getAccountAlertTitle(key)
  if (accountAlertTitle) return accountAlertTitle
  if (getBlockingOAuthStatusLabel(key, tick)) return getOAuthStatusTitle(key, tick)

  const reasons = getVisibleSchedulingReasons(key)
  if (reasons.length > 0) {
    return reasons.map((item) => {
      const ttl = item.ttl_seconds && item.ttl_seconds > 0 ? ` (${formatTTL(item.ttl_seconds)})` : ''
      const detail = item.detail ? ` - ${item.detail}` : ''
      return `${item.label}${ttl}${detail}`
    }).join('\n')
  }

  if (key.cooldown_reason) {
    const ttl = key.cooldown_ttl_seconds ? ` (${formatTTL(key.cooldown_ttl_seconds)})` : ''
    return `${formatCooldownReason(key.cooldown_reason)}${ttl}`
  }
  if (isPoolKeyCostExhausted(key)) return '超限'
  return getSchedulingBadgeLabel(key, tick)
}

export function formatTTL(seconds: number): string {
  if (seconds <= 0) return ''
  const m = Math.floor(seconds / 60)
  const s = seconds % 60
  return m > 0 ? `${m}m ${s}s` : `${s}s`
}

export function getRowClass(key: PoolKeyDetail, tick: number): string {
  const status = getSchedulingStatus(key, tick)
  if (!key.is_active || status === 'blocked') return 'bg-muted/50 opacity-60'
  return ''
}

export function getAuthTypeChipLabel(key: PoolKeyDetail): string {
  return getProviderAuthLabel(key)
}

export function getMobileOAuthTone(key: PoolKeyDetail, tick: number): PoolMobileTagTone | null {
  const oauthState = getVisibleOAuthState(key, tick)
  if (!oauthState) return null
  if (oauthState.isInvalid || oauthState.isExpired) return 'danger'
  if (oauthState.isExpiringSoon) return 'warning'
  return 'muted'
}

export function getMobileTagItems(key: PoolKeyDetail, tick: number): PoolMobileTagItem[] {
  const accountAlert = getAccountAlertLabel(key)
  const oauthState = getVisibleOAuthState(key, tick)
  const orgBadge = getOAuthOrgBadge(key)
  const planType = resolvePoolKeyPlanType(key)

  return buildPoolMobileTagItems({
    accountStatusLabel: compactPoolStatusLabel(accountAlert),
    accountStatusTone: accountAlert ? 'danger' : null,
    oauthStatusLabel: getOAuthStatusBadgeLabel(oauthState),
    oauthStatusTone: getMobileOAuthTone(key, tick),
    priorityLabel: `P${key.internal_priority ?? 50}`,
    authLabel: getAuthTypeChipLabel(key),
    planLabel: planType ? formatOAuthPlanType(planType) : null,
    orgLabel: orgBadge?.label ?? null,
    proxyLabel: key.proxy?.node_id ? '独立代理' : null,
  })
}

export function getMobileTagClass(item: PoolMobileTagItem): string {
  if (item.tone === 'danger') {
    return 'border-red-500/30 bg-red-500/10 text-red-700 dark:text-red-300'
  }
  if (item.tone === 'warning') {
    return 'border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300'
  }
  if (item.tone === 'accent') {
    return 'border-blue-500/30 bg-blue-500/10 text-blue-700 dark:text-blue-300'
  }
  if (item.tone === 'muted') {
    return 'border-border/60 bg-background/70 text-muted-foreground'
  }
  return 'border-border/60 bg-background/80 text-foreground/80'
}

export function getVisibleOAuthState(key: PoolKeyDetail, tick: number) {
  return getOAuthStatusDisplayWithFallback(key, tick)
}

export function getOAuthRefreshButtonTitle(key: PoolKeyDetail, tick: number): string {
  return resolveOAuthRefreshButtonTitle(key, tick)
}

export function getOAuthStatusTitle(key: PoolKeyDetail, tick: number): string {
  return resolveOAuthStatusTitle(key, tick)
}

export const _accountAlertCache = new WeakMap<PoolKeyDetail, string | null>()

export function getQuotaAlertSnapshotState(key: PoolKeyDetail): { label: string, title: string } | null {
  const quota = getQuotaSnapshot(key)
  if (!quota) return null

  const code = String(quota.code || '').trim().toLowerCase()
  if (!['banned', 'forbidden', 'quarantined', 'rate_limited', 'exhausted'].includes(code)) return null

  let label = String(quota.label || '').trim()
  if (!label) {
    if (code === 'banned') label = '账号封禁'
    else if (code === 'forbidden') label = '访问受限'
    else if (code === 'quarantined') label = '账号隔离'
    else if (code === 'rate_limited') label = '速率受限'
    else label = '额度耗尽'
  } else if (label === '账号已封禁' || label === '封禁') {
    label = '账号封禁'
  }

  const reason = String(quota.reason || '').trim()
  return {
    label,
    title: reason ? `${label}: ${reason}` : label,
  }
}

export function getAccountAlertLabel(key: PoolKeyDetail): string | null {
  const cached = _accountAlertCache.get(key)
  if (cached !== undefined) return cached

  let result: string | null = getAccountStatusDisplay(key).label
  const quotaAlert = getQuotaAlertSnapshotState(key)
  if (!result && quotaAlert) result = quotaAlert.label
  if (!result && !getQuotaSnapshot(key)) {
    const quotaText = getLegacyAccountQuotaText(key)
    if (quotaText === '账号已封禁' || quotaText === '封禁') result = '账号封禁'
    else if (quotaText === '访问受限') result = '访问受限'
  }

  _accountAlertCache.set(key, result)
  return result
}

export function getAccountAlertTitle(key: PoolKeyDetail): string {
  const label = getAccountAlertLabel(key)
  if (!label) return ''

  const accountTitle = getAccountStatusTitle(key)
  if (accountTitle) return accountTitle

  const quotaAlert = getQuotaAlertSnapshotState(key)
  if (quotaAlert?.title) return quotaAlert.title

  const quotaText = getLegacyAccountQuotaText(key)
  if (quotaText) return `${label}: ${quotaText}`
  return label
}

export function getQuotaTextClass(quotaText: string): string {
  if (quotaText.includes('封禁') || quotaText.includes('受限')) {
    return 'text-[11px] text-destructive leading-4'
  }
  return 'text-[11px] text-foreground/90 leading-4'
}

export function formatPoolScore(value: number | null | undefined): string {
  const n = Number(value)
  if (!Number.isFinite(n)) return '-'
  return n.toFixed(3)
}

export function formatPoolScoreReason(value: PoolKeyScore['score_reason'] | null | undefined): string {
  if (!value) return '暂无计算结果'
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

export function getPoolScoreHardStateLabel(value: PoolKeyScore['hard_state'] | null | undefined): string {
  if (!value) return '-'
  return poolScoreHardStateOptions.find(item => item.value === value)?.label || value
}

export function getPoolScoreProbeStatusLabel(value: PoolKeyScore['probe_status'] | null | undefined): string {
  if (!value) return '-'
  return poolScoreProbeStatusOptions.find(item => item.value === value)?.label || value
}

export function formatUnixSeconds(seconds: number | null | undefined): string {
  const raw = Number(seconds ?? 0)
  if (!Number.isFinite(raw) || raw <= 0) return '-'
  return formatRelativeTime(new Date(raw * 1000).toISOString())
}

export function formatRelativeTime(isoStr: string): string {
  const date = new Date(isoStr)
  const pad = (n: number) => String(n).padStart(2, '0')
  const M = pad(date.getMonth() + 1)
  const D = pad(date.getDate())
  const h = pad(date.getHours())
  const m = pad(date.getMinutes())
  return `${M}-${D} ${h}:${m}`
}

export function formatPoolKeyImportedAt(key: PoolKeyDetail): string {
  const value = key.imported_at || key.created_at
  return value ? formatRelativeTime(value) : '-'
}
