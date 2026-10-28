<template>
  <CardSection
    title="系统信息"
    description="当前系统版本和构建信息"
  >
    <div class="flex items-center gap-4">
      <div class="flex items-center gap-2">
        <Label class="text-sm font-medium text-muted-foreground">版本:</Label>
        <span
          v-if="systemVersion"
          class="text-sm font-mono"
        >
          {{ systemVersion }}
        </span>
        <span
          v-else
          class="text-sm text-muted-foreground"
        >
          加载中...
        </span>
      </div>
    </div>
  </CardSection>
</template>

<script setup lang="ts">
import Label from '@/components/ui/label.vue'
import { CardSection } from '@/components/layout'

defineProps<{
  systemVersion: string
}>()
</script>
