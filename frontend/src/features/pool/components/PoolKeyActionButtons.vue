<template>
  <div
    class="flex gap-0.5"
    :class="layout === 'desktop' ? 'justify-center' : 'items-center'"
  >
    <div
      v-for="actionId in actionIds"
      :key="`${poolKey.key_id}-${actionId}`"
      class="flex justify-center"
      :class="layout === 'mobile' ? 'min-w-0 flex-1' : ''"
    >
      <Button
        v-if="actionId === 'copy_or_download' && canExportOAuthCredential(poolKey)"
        variant="ghost"
        size="icon"
        class="h-7 w-7 shrink-0"
        title="下载 OAuth 授权文件"
        @click.stop="$emit('download')"
      >
        <Download class="w-3.5 h-3.5" />
      </Button>
      <Button
        v-else-if="actionId === 'copy_or_download' && poolKey.agent_identity !== true"
        variant="ghost"
        size="icon"
        class="h-7 w-7 shrink-0"
        title="复制密钥"
        @click.stop="$emit('copy')"
      >
        <Copy class="w-3.5 h-3.5" />
      </Button>
      <Button
        v-else-if="actionId === 'refresh_token'"
        variant="ghost"
        size="icon"
        class="h-7 w-7 shrink-0"
        :disabled="refreshingOauth || !canRefreshToken"
        :title="refreshButtonTitle"
        @click.stop="$emit('refreshOauth')"
      >
        <RefreshCw
          class="w-3.5 h-3.5"
          :class="{ 'animate-spin': refreshingOauth }"
        />
      </Button>
      <Button
        v-else-if="actionId === 'clear_cooldown'"
        variant="ghost"
        size="icon"
        class="h-7 w-7 shrink-0 text-muted-foreground hover:text-green-600"
        title="清除冷却"
        @click="$emit('clearCooldown')"
      >
        <RefreshCw class="w-3.5 h-3.5" />
      </Button>
      <Button
        v-else-if="actionId === 'permissions'"
        variant="ghost"
        size="icon"
        class="h-7 w-7 shrink-0"
        title="模型权限"
        @click="$emit('permissions')"
      >
        <Shield class="w-3.5 h-3.5" />
      </Button>
      <PoolKeyProxyPopover
        v-else-if="actionId === 'proxy'"
        :node-id="poolKey.proxy?.node_id || ''"
        :node-name="proxyNodeName"
        :open="proxyPopoverOpen"
        :saving="savingProxy"
        trigger-class="shrink-0"
        @update:open="(v: boolean) => $emit('update:proxyPopoverOpen', v)"
        @select="(v: string) => $emit('selectProxy', v)"
        @clear="$emit('clearProxy')"
      />
      <Button
        v-else-if="actionId === 'reset_cycle_stats'"
        variant="ghost"
        size="icon"
        class="h-7 w-7 shrink-0 text-muted-foreground hover:text-foreground"
        :disabled="resettingCycle"
        title="重置周期统计"
        :data-testid="layout === 'desktop' ? 'pool-reset-cycle-stats' : undefined"
        @click="$emit('resetCycleStats')"
      >
        <RotateCcw
          class="w-3.5 h-3.5"
          :class="{ 'animate-spin': resettingCycle }"
        />
      </Button>
      <Button
        v-else-if="actionId === 'edit'"
        variant="ghost"
        size="icon"
        class="h-7 w-7 shrink-0"
        title="编辑账号"
        @click="$emit('edit')"
      >
        <SquarePen class="w-3.5 h-3.5" />
      </Button>
      <Button
        v-else-if="actionId === 'toggle'"
        variant="ghost"
        size="icon"
        class="h-7 w-7 shrink-0 text-foreground hover:text-foreground"
        :disabled="toggling"
        :title="poolKey.is_active ? '禁用' : '启用'"
        :aria-label="poolKey.is_active ? '禁用账号' : '启用账号'"
        :data-testid="`pool-toggle-active-${layout}-${poolKey.key_id}`"
        @click="$emit('toggleActive')"
      >
        <Power class="w-3.5 h-3.5" />
      </Button>
      <Button
        v-else-if="actionId === 'delete'"
        variant="ghost"
        size="icon"
        class="h-7 w-7 shrink-0 text-destructive hover:text-destructive"
        :disabled="deleting"
        title="删除账号"
        @click="$emit('delete')"
      >
        <Trash2 class="w-3.5 h-3.5" />
      </Button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { Copy, Download, Power, RefreshCw, RotateCcw, Shield, SquarePen, Trash2 } from 'lucide-vue-next'
import { Button } from '@/components/ui'
import type { PoolKeyDetail } from '@/api/endpoints/pool'
import { canExportOAuthCredential } from '@/features/providers/utils/providerKeyAuth'
import type { PoolMobileActionId } from '../utils/poolMobilePresentation'
import PoolKeyProxyPopover from './PoolKeyProxyPopover.vue'

export type PoolKeyActionId = PoolMobileActionId

/**
 * 号池密钥的操作按钮组。按 actionIds 顺序渲染，桌面表格与移动卡片共用，
 * 移动布局下每个按钮占等宽一格。
 */
withDefaults(defineProps<{
  poolKey: PoolKeyDetail
  layout: 'desktop' | 'mobile'
  actionIds: PoolKeyActionId[]
  canRefreshToken?: boolean
  refreshButtonTitle?: string
  refreshingOauth?: boolean
  resettingCycle?: boolean
  toggling?: boolean
  deleting?: boolean
  savingProxy?: boolean
  proxyNodeName?: string | null
  proxyPopoverOpen?: boolean
}>(), {
  canRefreshToken: false,
  refreshButtonTitle: '',
  refreshingOauth: false,
  resettingCycle: false,
  toggling: false,
  deleting: false,
  savingProxy: false,
  proxyNodeName: null,
  proxyPopoverOpen: false,
})

defineEmits<{
  download: []
  copy: []
  refreshOauth: []
  clearCooldown: []
  permissions: []
  resetCycleStats: []
  edit: []
  toggleActive: []
  delete: []
  'update:proxyPopoverOpen': [open: boolean]
  selectProxy: [nodeId: string]
  clearProxy: []
}>()
</script>
