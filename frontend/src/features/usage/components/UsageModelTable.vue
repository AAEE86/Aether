<template>
  <UsageStatsTable
    title="按模型分析"
    name-header="模型"
    empty-text="暂无模型统计数据"
    :rows="rows"
    :extra-columns="MODEL_STATS_COLUMNS"
    :is-admin="isAdmin"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import UsageStatsTable from './UsageStatsTable.vue'
import { MODEL_STATS_COLUMNS, modelStatsRows } from '../utils/usageStatsRows'
import type { EnhancedModelStatsItem } from '../types'

const props = defineProps<{
  data: EnhancedModelStatsItem[]
  isAdmin: boolean
}>()

const rows = computed(() => modelStatsRows(props.data))
</script>
