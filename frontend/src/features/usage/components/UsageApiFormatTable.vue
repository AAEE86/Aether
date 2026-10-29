<template>
  <UsageStatsTable
    title="按API格式分析"
    name-header="API格式"
    empty-text="暂无API格式统计数据"
    :rows="rows"
    :extra-columns="API_FORMAT_STATS_COLUMNS"
    :is-admin="isAdmin"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import UsageStatsTable from './UsageStatsTable.vue'
import { API_FORMAT_STATS_COLUMNS, apiFormatStatsRows } from '../utils/usageStatsRows'
import type { ApiFormatStatsItem } from '../types'

const props = defineProps<{
  data: ApiFormatStatsItem[]
  isAdmin: boolean
}>()

const rows = computed(() => apiFormatStatsRows(props.data))
</script>
