<template>
  <HealthMonitorSection
    :title="title"
    description="仅展示活跃提供商，点击详情查看该提供商关联的端点与模型健康"
    :icon="Server"
    empty-title="暂无提供商健康数据"
    empty-description="当前没有提供商产生请求记录"
    entity-label="提供商"
    error-message="加载提供商健康监控数据失败"
    :load="loadProviderMonitors"
    @view-details="$emit('viewDetails', $event)"
    @summary-updated="$emit('summaryUpdated', $event)"
  />
</template>

<script setup lang="ts">
import { Server } from 'lucide-vue-next'
import HealthMonitorSection, { type HealthMonitorSectionData } from './HealthMonitorSection.vue'
import { getProviderStatusMonitor } from '@/api/endpoints/health'
import type { ProviderStatusMonitor } from '@/api/endpoints/types'
import {
  formatCompactNumber,
  type HealthMonitorDetailTarget,
  type HealthMonitorSectionSummary,
} from './health-monitor-utils'

withDefaults(defineProps<{
  title?: string
}>(), {
  title: '提供商健康监控'
})

defineEmits<{
  viewDetails: [target: HealthMonitorDetailTarget]
  summaryUpdated: [summary: HealthMonitorSectionSummary]
}>()

async function loadProviderMonitors(lookbackHours: number): Promise<HealthMonitorSectionData<ProviderStatusMonitor>> {
  const data = await getProviderStatusMonitor({
    lookback_hours: lookbackHours,
    provider_limit: 50,
    per_provider_model_limit: 12,
    per_model_limit: 100
  })
  const providers = (data.providers || []).filter(provider => provider.total_attempts > 0)
  return {
    generatedAt: data.generated_at,
    items: providers.map(provider => ({
      key: provider.provider_id,
      title: provider.provider_name,
      metaText: `${provider.model_count} 个模型 / ${formatCompactNumber(provider.total_attempts)} 次请求`,
      entityName: provider.provider_name,
      monitor: provider,
      raw: provider,
      detail: {
        kind: 'provider',
        value: provider.provider_name,
        title: provider.provider_name,
        metaText: null,
        totalAttempts: provider.total_attempts,
        successCount: provider.success_count,
        failedCount: provider.failed_count,
        successRate: provider.success_rate,
        avgLatencyMs: provider.avg_latency_ms,
        avgFirstByteMs: provider.avg_first_byte_ms,
        avgTps: provider.avg_tps,
        timeline: provider.timeline || null,
        timelineDetails: provider.timeline_details || null,
        timeRangeStart: provider.time_range_start || null,
        timeRangeEnd: provider.time_range_end || null
      },
    })),
  }
}
</script>
