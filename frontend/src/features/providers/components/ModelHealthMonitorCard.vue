<template>
  <HealthMonitorSection
    :title="title"
    description="基于真实请求统计模型可用率、平均耗时、平均TTFB 与平均速度"
    :icon="Bot"
    empty-title="暂无模型健康监控数据"
    empty-description="模型尚未产生请求记录"
    entity-label="模型"
    error-message="加载模型健康监控数据失败"
    :load="loadModelMonitors"
    @view-details="$emit('viewDetails', $event)"
    @summary-updated="$emit('summaryUpdated', $event)"
  />
</template>

<script setup lang="ts">
import { Bot } from 'lucide-vue-next'
import HealthMonitorSection, { type HealthMonitorSectionData } from './HealthMonitorSection.vue'
import { getModelStatusMonitor, getPublicModelStatusMonitor } from '@/api/endpoints/health'
import type { ModelStatusMonitor } from '@/api/endpoints/types'
import {
  formatCompactNumber,
  type HealthMonitorDetailTarget,
  type HealthMonitorSectionSummary,
} from './health-monitor-utils'

const props = withDefaults(defineProps<{
  title?: string
  isAdmin?: boolean
  showProviderInfo?: boolean
}>(), {
  title: '模型健康监控',
  isAdmin: false,
  showProviderInfo: false,
})

defineEmits<{
  viewDetails: [target: HealthMonitorDetailTarget]
  summaryUpdated: [summary: HealthMonitorSectionSummary]
}>()

function getModelMetaText(monitor: ModelStatusMonitor) {
  const attempts = `${formatCompactNumber(monitor.total_attempts)} 次请求`
  if (props.showProviderInfo && typeof monitor.provider_count === 'number') {
    return `${monitor.provider_count} 个提供商 / ${attempts}`
  }
  return attempts
}

function getModelDetailMetaText(monitor: ModelStatusMonitor) {
  if (props.showProviderInfo && typeof monitor.provider_count === 'number') {
    return `${monitor.provider_count} 个提供商`
  }
  return null
}

async function loadModelMonitors(lookbackHours: number): Promise<HealthMonitorSectionData<ModelStatusMonitor>> {
  const params = { lookback_hours: lookbackHours, model_limit: 12, per_model_limit: 100 }
  const data = props.isAdmin
    ? await getModelStatusMonitor(params)
    : await getPublicModelStatusMonitor(params)
  return {
    generatedAt: data.generated_at,
    items: (data.models || []).map(monitor => ({
      key: monitor.model,
      title: monitor.display_name || monitor.model,
      metaText: getModelMetaText(monitor),
      entityName: monitor.model,
      monitor,
      raw: monitor,
      detail: {
        kind: 'model',
        value: monitor.model,
        title: monitor.display_name || monitor.model,
        metaText: getModelDetailMetaText(monitor),
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
        timeRangeEnd: monitor.time_range_end || null,
      },
    })),
  }
}
</script>
