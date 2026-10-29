<template>
  <UsageStatsTable
    title="按提供商分析"
    name-header="提供商"
    empty-text="暂无提供商统计数据"
    :rows="rows"
    :extra-columns="PROVIDER_STATS_COLUMNS"
    :is-admin="isAdmin"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import UsageStatsTable from './UsageStatsTable.vue'
import { PROVIDER_STATS_COLUMNS, providerStatsRows } from '../utils/usageStatsRows'
import type { ProviderStatsItem } from '../types'

const props = defineProps<{
  data: ProviderStatsItem[]
  isAdmin: boolean
}>()

const rows = computed(() => providerStatsRows(props.data))
</script>
