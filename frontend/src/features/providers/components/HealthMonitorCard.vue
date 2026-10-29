<template>
  <HealthMonitorSection
    :title="title"
    description="基于真实请求统计端点可用率、平均耗时、平均TTFB 与平均速度"
    :icon="Activity"
    empty-title="暂无健康监控数据"
    empty-description="端点尚未产生请求记录"
    entity-label="端点"
    error-message="加载健康监控数据失败"
    :load="loadEndpointMonitors"
    @view-details="$emit('viewDetails', $event)"
    @summary-updated="$emit('summaryUpdated', $event)"
  >
    <template #timeline="{ item, lookbackHours }">
      <div class="mt-2">
        <EndpointHealthTimeline
          :monitor="item.raw"
          :lookback-hours="lookbackHours"
        />
      </div>
    </template>
  </HealthMonitorSection>
</template>

<script setup lang="ts">
import { Activity } from 'lucide-vue-next'
import HealthMonitorSection, { type HealthMonitorSectionData } from './HealthMonitorSection.vue'
import EndpointHealthTimeline from './EndpointHealthTimeline.vue'
import { getEndpointStatusMonitor, getPublicEndpointStatusMonitor } from '@/api/endpoints/health'
import type { EndpointStatusMonitor, PublicEndpointStatusMonitor } from '@/api/endpoints/types'
import { formatApiFormat } from '@/api/endpoints/types/api-format'
import {
  formatCompactNumber,
  type HealthMonitorDetailTarget,
  type HealthMonitorSectionSummary,
} from './health-monitor-utils'

type EndpointMonitor = EndpointStatusMonitor | PublicEndpointStatusMonitor

const props = withDefaults(defineProps<{
  title?: string
  isAdmin?: boolean
  showProviderInfo?: boolean
}>(), {
  title: '健康监控',
  isAdmin: false,
  showProviderInfo: false
})

defineEmits<{
  viewDetails: [target: HealthMonitorDetailTarget]
  summaryUpdated: [summary: HealthMonitorSectionSummary]
}>()

function hasProviderInfo(monitor: EndpointMonitor): monitor is EndpointStatusMonitor {
  return 'provider_count' in monitor && typeof monitor.provider_count === 'number'
}

function hasApiPath(monitor: EndpointMonitor): monitor is PublicEndpointStatusMonitor {
  return 'api_path' in monitor && typeof monitor.api_path === 'string' && monitor.api_path.length > 0
}

function getEndpointMetaText(monitor: EndpointMonitor) {
  const attempts = `${formatCompactNumber(monitor.total_attempts)} 次请求`
  if (props.showProviderInfo && hasProviderInfo(monitor)) {
    return `${monitor.provider_count} 个提供商 / ${monitor.key_count} 个密钥 / ${attempts}`
  }
  if (hasApiPath(monitor)) {
    return `${monitor.api_path} / ${attempts}`
  }
  return attempts
}

function getEndpointDetailMetaText(monitor: EndpointMonitor) {
  if (props.showProviderInfo && hasProviderInfo(monitor)) {
    return `${monitor.provider_count} 个提供商 / ${monitor.key_count} 个密钥`
  }
  if (hasApiPath(monitor)) {
    return monitor.api_path
  }
  return null
}

async function loadEndpointMonitors(lookbackHours: number): Promise<HealthMonitorSectionData<EndpointMonitor>> {
  const params = { lookback_hours: lookbackHours, per_format_limit: 100 }
  const data = props.isAdmin
    ? await getEndpointStatusMonitor(params)
    : await getPublicEndpointStatusMonitor(params)
  const monitors: EndpointMonitor[] = (data.formats || []).filter(monitor => monitor.total_attempts > 0)
  return {
    generatedAt: data.generated_at,
    items: monitors.map((monitor, index) => ({
      key: `${monitor.api_format}-${index}`,
      title: formatApiFormat(monitor.api_format),
      metaText: getEndpointMetaText(monitor),
      entityName: formatApiFormat(monitor.api_format),
      monitor,
      raw: monitor,
      detail: {
        kind: 'endpoint',
        value: monitor.api_format,
        title: formatApiFormat(monitor.api_format),
        metaText: getEndpointDetailMetaText(monitor),
        totalAttempts: monitor.total_attempts,
        successCount: monitor.success_count,
        failedCount: monitor.failed_count,
        successRate: monitor.success_rate,
        avgLatencyMs: monitor.avg_latency_ms,
        avgFirstByteMs: monitor.avg_first_byte_ms,
        avgTps: monitor.avg_tps,
        timeline: monitor.timeline || null,
        timelineDetails: monitor.timeline_details || null,
        timeRangeStart: monitor.time_range_start || null,
        timeRangeEnd: monitor.time_range_end || null
      },
    })),
  }
}
</script>
