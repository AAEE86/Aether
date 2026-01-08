<template>
  <Card class="overflow-hidden mb-6">
    <div class="p-4 border-b border-border/60">
      <div class="flex items-center justify-between">
        <h3 class="text-sm font-semibold flex items-center gap-2">
          <span>共享密钥</span>
          <Badge variant="secondary" class="font-normal text-xs">Provider Level</Badge>
        </h3>
        <div class="flex items-center gap-2">
          <Button
            v-if="hasUnhealthyKeys"
            variant="ghost"
            size="icon"
            class="h-8 w-8 text-green-600"
            title="恢复所有密钥健康状态"
            :disabled="recoveringAll"
            @click="handleRecoverAllKeys"
          >
            <Loader2
              v-if="recoveringAll"
              class="w-3.5 h-3.5 animate-spin"
            />
            <RefreshCw
              v-else
              class="w-3.5 h-3.5"
            />
          </Button>
          <Button
            variant="outline"
            size="sm"
            class="h-8"
            @click="$emit('add-key')"
          >
            <Plus class="w-3.5 h-3.5 mr-1.5" />
            添加共享密钥
          </Button>
        </div>
      </div>
    </div>

    <!-- 加载状态 -->
    <div
      v-if="loading"
      class="flex items-center justify-center py-8"
    >
      <div class="animate-spin rounded-full h-6 w-6 border-b-2 border-primary" />
    </div>

    <!-- 密钥列表 -->
    <div
      v-else-if="keys.length > 0"
      class="p-4 space-y-2"
    >
      <div
        v-for="key in keys"
        :key="key.id"
        draggable="true"
        class="p-3 bg-background rounded-md border transition-all duration-150 group"
        :class="{
          'border-border/40 hover:border-border/80': dragState.targetKeyId !== key.id,
          'border-primary border-2 bg-primary/5': dragState.targetKeyId === key.id,
          'opacity-50': dragState.draggedKeyId === key.id,
          'cursor-grabbing': dragState.isDragging
        }"
        @dragstart="handleDragStart($event, key)"
        @dragend="handleDragEnd"
        @dragover="handleDragOver($event, key)"
        @dragleave="handleDragLeave"
        @drop="handleDrop($event, key)"
      >
        <div class="flex items-center justify-between mb-2">
          <div class="flex items-center gap-2 flex-1 min-w-0">
            <!-- 拖动手柄 -->
            <div
              class="cursor-grab active:cursor-grabbing text-muted-foreground/50 hover:text-muted-foreground"
              title="拖动排序"
            >
              <GripVertical class="w-4 h-4" />
            </div>
            <div class="min-w-0">
              <div class="flex items-center gap-1.5">
                <span class="text-xs font-medium truncate">{{ key.name || '未命名密钥' }}</span>
                <Badge
                  :variant="key.is_active ? 'default' : 'secondary'"
                  class="text-[10px] px-1.5 py-0 shrink-0"
                >
                  {{ key.is_active ? '活跃' : '禁用' }}
                </Badge>
              </div>
              <div class="flex items-center gap-1">
                <span class="text-[10px] font-mono text-muted-foreground truncate max-w-[180px]">
                  {{ revealedKeys.has(key.id) ? revealedKeys.get(key.id) : key.api_key_masked }}
                </span>
                <Button
                  variant="ghost"
                  size="icon"
                  class="h-5 w-5 shrink-0"
                  :title="revealedKeys.has(key.id) ? '隐藏密钥' : '显示密钥'"
                  :disabled="revealingKeyId === key.id"
                  @click.stop="toggleKeyReveal(key)"
                >
                  <Loader2
                    v-if="revealingKeyId === key.id"
                    class="w-3 h-3 animate-spin"
                  />
                  <EyeOff
                    v-else-if="revealedKeys.has(key.id)"
                    class="w-3 h-3"
                  />
                  <Eye
                    v-else
                    class="w-3 h-3"
                  />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  class="h-5 w-5 shrink-0"
                  title="复制密钥"
                  @click.stop="copyFullKey(key)"
                >
                  <Copy class="w-3 h-3" />
                </Button>
              </div>
            </div>
            <!-- Metrics -->
            <div class="flex items-center gap-1.5 ml-auto shrink-0">
              <div
                v-if="key.health_score !== undefined"
                class="flex items-center gap-1"
              >
                <div class="w-12 h-1 bg-muted/80 rounded-full overflow-hidden">
                  <div
                    class="h-full transition-all duration-300"
                    :class="getHealthScoreBarColor(key.health_score || 0)"
                    :style="{ width: `${(key.health_score || 0) * 100}%` }"
                  />
                </div>
                <span
                  class="text-[10px] font-bold tabular-nums w-[30px] text-right"
                  :class="getHealthScoreColor(key.health_score || 0)"
                >
                  {{ ((key.health_score || 0) * 100).toFixed(0) }}%
                </span>
              </div>
              <Badge
                v-if="key.circuit_breaker_open"
                variant="destructive"
                class="text-[10px] px-1.5 py-0"
              >
                熔断
              </Badge>
            </div>
          </div>

          <!-- Actions -->
          <div class="flex items-center gap-1 ml-2">
            <Button
              v-if="key.circuit_breaker_open || (key.health_score !== undefined && key.health_score < 0.5)"
              variant="ghost"
              size="icon"
              class="h-7 w-7 text-green-600"
              title="刷新健康状态"
              @click="handleRecoverKey(key)"
            >
              <RefreshCw class="w-3 h-3" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              class="h-7 w-7"
              title="配置允许的模型"
              @click="$emit('config-models', key)"
            >
              <Layers class="w-3 h-3" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              class="h-7 w-7"
              title="编辑密钥"
              @click="$emit('edit-key', key)"
            >
              <Edit class="w-3 h-3" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              class="h-7 w-7"
              :disabled="togglingKeyId === key.id"
              :title="key.is_active ? '点击停用' : '点击启用'"
              @click="toggleKeyActive(key)"
            >
              <Power class="w-3 h-3" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              class="h-7 w-7"
              title="删除密钥"
              @click="$emit('delete-key', key)"
            >
              <Trash2 class="w-3 h-3" />
            </Button>
          </div>
        </div>

        <!-- Details -->
        <div class="flex items-center text-[11px]">
          <!-- 左侧固定信息 -->
          <div class="flex items-center gap-2">
            <!-- 可点击编辑的优先级 -->
            <span
              v-if="editingPriorityKey !== key.id"
              class="text-muted-foreground cursor-pointer hover:text-foreground hover:bg-muted/50 px-1 rounded transition-colors"
              title="点击编辑优先级，数字越小优先级越高"
              @click="startEditPriority(key)"
            >
              P {{ key.internal_priority }}
            </span>
            <!-- 编辑模式 -->
            <span
              v-else
              class="flex items-center gap-1"
            >
              <span class="text-muted-foreground">P</span>
              <input
                ref="priorityInput"
                v-model.number="editingPriorityValue"
                type="number"
                class="w-12 h-5 px-1 text-[11px] border rounded bg-background focus:outline-none focus:ring-1 focus:ring-primary"
                min="0"
                @keyup.enter="savePriority(key)"
                @keyup.escape="cancelEditPriority"
                @blur="savePriority(key)"
              >
            </span>
            <span
              class="text-muted-foreground"
              title="成本倍率，实际成本 = 模型价格 × 倍率"
            >
              {{ key.rate_multiplier }}x
            </span>
            <span
              v-if="key.success_rate !== undefined"
              class="text-muted-foreground"
              title="成功率 = 成功次数 / 总请求数"
            >
              {{ (key.success_rate * 100).toFixed(1) }}% ({{ key.success_count }}/{{ key.request_count }})
            </span>
          </div>
          <!-- 右侧动态信息 -->
          <div class="flex items-center gap-2 ml-auto">
            <span
              v-if="key.next_probe_at"
              class="text-amber-600 dark:text-amber-400"
              title="熔断器探测恢复时间"
            >
              {{ formatProbeTime(key.next_probe_at) }}探测
            </span>
            <span
              v-if="key.rate_limit"
              class="text-muted-foreground"
              title="每分钟请求数限制"
            >
              {{ key.rate_limit }}rpm
            </span>
            <span
              v-if="key.max_concurrent || key.is_adaptive"
              class="text-muted-foreground"
              :title="key.is_adaptive ? `自适应并发限制（学习值: ${key.learned_max_concurrent ?? '未学习'}）` : `固定并发限制: ${key.max_concurrent}`"
            >
              {{ key.is_adaptive ? '自适应' : '固定' }}并发: {{ key.is_adaptive ? (key.learned_max_concurrent ?? '学习中') : key.max_concurrent }}
            </span>
          </div>
        </div>
      </div>
    </div>

    <!-- Empty State -->
    <div
      v-else
      class="p-6 text-center text-muted-foreground"
    >
      <Key class="w-8 h-8 mx-auto mb-2 opacity-50" />
      <p class="text-sm">
        暂无共享密钥
      </p>
      <p class="text-xs mt-1">
        共享密钥可被该提供商下的所有端点使用
      </p>
    </div>
  </Card>
</template>

<script setup lang="ts">
import { ref, onMounted, watch, computed } from 'vue'
import {
    Plus, Key, Edit, Trash2, Power, Layers, Copy, Loader2, Eye, EyeOff, RefreshCw, GripVertical
} from 'lucide-vue-next'
import { Card, Button, Badge } from '@/components/ui'
import { useToast } from '@/composables/useToast'
import { useClipboard } from '@/composables/useClipboard'
import {
    getProviderKeys,
    updateProviderKey,
    revealProviderKey,
    batchUpdateProviderKeyPriority
} from '@/api/endpoints/keys'
import { recoverKeyHealth } from '@/api/endpoints'
import type { EndpointAPIKey } from '@/api/endpoints/types'

const props = defineProps<{
    provider: any
}>()

const emit = defineEmits<{
    'add-key': []
    'edit-key': [key: EndpointAPIKey]
    'delete-key': [key: EndpointAPIKey]
    'config-models': [key: EndpointAPIKey]
}>()

const { success, error: showError } = useToast()
const { copyToClipboard } = useClipboard()

const loading = ref(false)
const keys = ref<EndpointAPIKey[]>([])
const togglingKeyId = ref<string | null>(null)
const revealedKeys = ref<Map<string, string>>(new Map())
const revealingKeyId = ref<string | null>(null)
const recoveringAll = ref(false)

// 拖动排序相关状态
const dragState = ref({
  isDragging: false,
  draggedKeyId: null as string | null,
  targetKeyId: null as string | null
})

// 点击编辑优先级相关状态
const editingPriorityKey = ref<string | null>(null)
const editingPriorityValue = ref<number>(0)

// 计算是否有不健康的密钥
const hasUnhealthyKeys = computed(() => {
  return keys.value.some((key: EndpointAPIKey) =>
    key.circuit_breaker_open ||
    (key.health_score !== undefined && key.health_score < 1)
  )
})

async function loadKeys() {
    if (!props.provider?.id) return
    loading.value = true
    try {
        keys.value = await getProviderKeys(props.provider.id)
    } catch (err: any) {
        showError(err.message || '加载共享密钥失败', '错误')
    } finally {
        loading.value = false
    }
}

// Expose refresh method
defineExpose({ refresh: loadKeys })

onMounted(loadKeys)
watch(() => props.provider?.id, loadKeys)

// Actions
async function toggleKeyActive(key: EndpointAPIKey) {
    if (togglingKeyId.value) return
    togglingKeyId.value = key.id
    try {
        const newStatus = !key.is_active
        await updateProviderKey(key.id, { is_active: newStatus })
        key.is_active = newStatus
        success(newStatus ? '密钥已启用' : '密钥已停用')
    } catch (err: any) {
        showError('更新状态失败', '错误')
    } finally {
        togglingKeyId.value = null
    }
}

async function toggleKeyReveal(key: EndpointAPIKey) {
  if (revealingKeyId.value === key.id) return

  if (revealedKeys.value.has(key.id)) {
    revealedKeys.value.delete(key.id)
    return
  }

  revealingKeyId.value = key.id
  try {
    const { api_key } = await revealProviderKey(key.id)
    revealedKeys.value.set(key.id, api_key)
  } catch (err: any) {
    showError('无法获取密钥完整内容', '错误')
  } finally {
    revealingKeyId.value = null
  }
}

async function copyFullKey(key: EndpointAPIKey) {
  let text = key.api_key_masked
  if (revealedKeys.value.has(key.id)) {
    text = revealedKeys.value.get(key.id)!
  } else {
    // If not revealed, try to fast reveal and copy
    try {
        const { api_key } = await revealProviderKey(key.id)
        text = api_key
    } catch (e) {
        showError('无法复制密钥', '错误')
        return
    }
  }
  await copyToClipboard(text)
}

function getHealthScoreBarColor(score: number): string {
  if (score >= 0.8) return 'bg-green-500 dark:bg-green-400'
  if (score >= 0.5) return 'bg-yellow-500 dark:bg-yellow-400'
  return 'bg-red-500 dark:bg-red-400'
}

function getHealthScoreColor(score: number): string {
  if (score >= 0.8) return 'text-green-600 dark:text-green-400'
  if (score >= 0.5) return 'text-yellow-600 dark:text-yellow-400'
  return 'text-red-600 dark:text-red-400'
}

// 格式化探测时间
function formatProbeTime(probeTime: string): string {
  if (!probeTime) return '-'
  const now = new Date()
  const probe = new Date(probeTime)
  const diffMs = probe.getTime() - now.getTime()

  if (diffMs < 0) return '待探测'

  const diffMinutes = Math.floor(diffMs / 60000)
  const diffHours = Math.floor(diffMinutes / 60)
  const diffDays = Math.floor(diffHours / 24)

  if (diffDays > 0) return `${diffDays}天后`
  if (diffHours > 0) return `${diffHours}小时后`
  if (diffMinutes > 0) return `${diffMinutes}分钟后`
  return '即将探测'
}

// 恢复单个密钥健康状态
async function handleRecoverKey(key: EndpointAPIKey) {
  try {
    const result = await recoverKeyHealth(key.id)
    success(result.message || 'Key已完全恢复')
    await loadKeys()
  } catch (err: any) {
    showError('Key恢复失败', '错误')
  }
}

// 批量恢复所有不健康密钥
async function handleRecoverAllKeys() {
  const keysToRecover = keys.value.filter((key: EndpointAPIKey) =>
    key.circuit_breaker_open ||
    (key.health_score !== undefined && key.health_score < 1)
  )

  if (keysToRecover.length === 0) {
    success('所有密钥已处于健康状态')
    return
  }

  recoveringAll.value = true
  let successCount = 0
  let failCount = 0

  try {
    for (const key of keysToRecover) {
      try {
        await recoverKeyHealth(key.id)
        successCount++
      } catch {
        failCount++
      }
    }

    if (failCount === 0) {
      success(`已恢复 ${successCount} 个密钥的健康状态`)
    } else {
      success(`恢复完成: ${successCount} 成功, ${failCount} 失败`)
    }

    await loadKeys()
  } finally {
    recoveringAll.value = false
  }
}

// ===== 拖动排序处理 =====
function handleDragStart(event: DragEvent, key: EndpointAPIKey) {
  dragState.value.isDragging = true
  dragState.value.draggedKeyId = key.id
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = 'move'
  }
}

function handleDragEnd() {
  dragState.value.isDragging = false
  dragState.value.draggedKeyId = null
  dragState.value.targetKeyId = null
}

function handleDragOver(event: DragEvent, targetKey: EndpointAPIKey) {
  event.preventDefault()
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = 'move'
  }
  if (dragState.value.draggedKeyId !== targetKey.id) {
    dragState.value.targetKeyId = targetKey.id
  }
}

function handleDragLeave() {
  dragState.value.targetKeyId = null
}

async function handleDrop(event: DragEvent, targetKey: EndpointAPIKey) {
  event.preventDefault()

  const draggedKeyId = dragState.value.draggedKeyId
  if (!draggedKeyId || draggedKeyId === targetKey.id) {
    handleDragEnd()
    return
  }

  const keysList = [...keys.value]
  const draggedIndex = keysList.findIndex(k => k.id === draggedKeyId)
  const targetIndex = keysList.findIndex(k => k.id === targetKey.id)

  if (draggedIndex === -1 || targetIndex === -1) {
    handleDragEnd()
    return
  }

  // 记录原始优先级分组（排除被拖动的密钥）
  const originalGroups = new Map<number, string[]>()
  for (const key of keysList) {
    if (key.id === draggedKeyId) continue
    const priority = key.internal_priority ?? 0
    if (!originalGroups.has(priority)) {
      originalGroups.set(priority, [])
    }
    originalGroups.get(priority)!.push(key.id)
  }

  // 重排数组
  const [removed] = keysList.splice(draggedIndex, 1)
  keysList.splice(targetIndex, 0, removed)
  keys.value = keysList

  // 按新顺序为每个组分配新的优先级
  const priorities: { key_id: string; internal_priority: number }[] = []
  const groupNewPriority = new Map<number, number>()
  let currentPriority = 0

  for (const key of keysList) {
    if (key.id === draggedKeyId) {
      priorities.push({ key_id: key.id, internal_priority: currentPriority })
      currentPriority++
    } else {
      const originalPriority = key.internal_priority ?? 0
      
      if (groupNewPriority.has(originalPriority)) {
        priorities.push({ key_id: key.id, internal_priority: groupNewPriority.get(originalPriority)! })
      } else {
        groupNewPriority.set(originalPriority, currentPriority)
        priorities.push({ key_id: key.id, internal_priority: currentPriority })
        currentPriority++
      }
    }
  }

  handleDragEnd()

  // 调用 API 批量更新
  try {
    await batchUpdateProviderKeyPriority(props.provider.id, priorities)
    success('优先级已更新')
    await loadKeys()
  } catch (err: any) {
    showError('更新优先级失败', '错误')
    await loadKeys()
  }
}

// ===== 点击编辑优先级 =====
function startEditPriority(key: EndpointAPIKey) {
  editingPriorityKey.value = key.id
  editingPriorityValue.value = key.internal_priority ?? 0
}

function cancelEditPriority() {
  editingPriorityKey.value = null
}

async function savePriority(key: EndpointAPIKey) {
  const keyId = editingPriorityKey.value
  const newPriority = editingPriorityValue.value

  if (!keyId || newPriority < 0) {
    cancelEditPriority()
    return
  }

  if (key.internal_priority === newPriority) {
    cancelEditPriority()
    return
  }

  cancelEditPriority()

  try {
    await updateProviderKey(keyId, { internal_priority: newPriority })
    success('优先级已更新')
    const keyToUpdate = keys.value.find((k: EndpointAPIKey) => k.id === keyId)
    if (keyToUpdate) {
      keyToUpdate.internal_priority = newPriority
    }
    keys.value.sort((a: EndpointAPIKey, b: EndpointAPIKey) => (a.internal_priority ?? 0) - (b.internal_priority ?? 0))
  } catch (err: any) {
    showError('更新优先级失败', '错误')
  }
}
</script>
