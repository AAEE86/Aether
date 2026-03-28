import type {
  ClaudeCodeAdvancedConfig,
  PoolAdvancedConfig,
} from '@/api/endpoints/types/provider'

export type PoolAdvancedHighlightTone = 'default' | 'success' | 'warning' | 'muted'

export interface PoolAdvancedHighlight {
  key: string
  label: string
  value: string
  tone: PoolAdvancedHighlightTone
}

function formatDurationFromSeconds(value: number | null | undefined): string | null {
  if (!Number.isFinite(value ?? NaN) || Number(value) <= 0) return null

  const seconds = Number(value)
  if (seconds % 3600 === 0) {
    return `${seconds / 3600} 小时`
  }
  if (seconds % 60 === 0) {
    return `${seconds / 60} 分钟`
  }
  return `${seconds} 秒`
}

function formatClaudeSessionValue(config: ClaudeCodeAdvancedConfig | null | undefined): string {
  const maxSessions = config?.max_sessions
  const idleTimeout = config?.session_idle_timeout_minutes ?? 5

  if (maxSessions == null) {
    return `不限 / 空闲 ${idleTimeout} 分钟`
  }
  return `最多 ${maxSessions} 会话 / 空闲 ${idleTimeout} 分钟`
}

export function buildPoolAdvancedHighlights(
  poolConfig: PoolAdvancedConfig | null | undefined,
  claudeConfig: ClaudeCodeAdvancedConfig | null | undefined,
  isClaudeCode: boolean,
): PoolAdvancedHighlight[] {
  const healthEnabled = poolConfig?.health_policy_enabled !== false
  const probingEnabled = poolConfig?.probing_enabled === true
  const probingInterval = poolConfig?.probing_interval_minutes
  const costWindow = formatDurationFromSeconds(poolConfig?.cost_window_seconds)
  const batchConcurrency = poolConfig?.batch_concurrency

  const items: PoolAdvancedHighlight[] = [
    {
      key: 'health',
      label: '健康策略',
      value: healthEnabled ? '已开启' : '已关闭',
      tone: healthEnabled ? 'success' : 'warning',
    },
    {
      key: 'probing',
      label: '主动探测',
      value: probingEnabled && Number.isFinite(probingInterval ?? NaN)
        ? `${probingInterval} 分钟/次`
        : (probingEnabled ? '已启用' : '未启用'),
      tone: probingEnabled ? 'default' : 'muted',
    },
    {
      key: 'cost',
      label: '成本窗口',
      value: costWindow ?? '未配置',
      tone: costWindow ? 'default' : 'muted',
    },
    {
      key: 'batch',
      label: '批量并发',
      value: Number.isFinite(batchConcurrency ?? NaN) ? `${batchConcurrency} 路` : '默认',
      tone: Number.isFinite(batchConcurrency ?? NaN) ? 'default' : 'muted',
    },
  ]

  if (isClaudeCode) {
    items.push({
      key: 'claude-session',
      label: '会话控制',
      value: formatClaudeSessionValue(claudeConfig),
      tone: 'default',
    })
  }

  return items
}
