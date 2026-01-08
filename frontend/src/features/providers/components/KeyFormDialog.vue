<template>
  <Dialog
    :model-value="isOpen"
    :title="isEditMode ? '编辑密钥' : '添加密钥'"
    :description="isEditMode ? '修改 API 密钥配置' : '为端点添加新的 API 密钥'"
    :icon="isEditMode ? SquarePen : Key"
    size="2xl"
    @update:model-value="handleDialogUpdate"
  >
    <form
      class="space-y-5"
      autocomplete="off"
      @submit.prevent="handleSave"
    >
      <!-- 基本信息 -->
      <div class="space-y-3">
        <h3 class="text-sm font-medium border-b pb-2">
          基本信息
        </h3>
        <div class="grid grid-cols-2 gap-4">
          <div>
            <Label :for="keyNameInputId">密钥名称 *</Label>
            <Input
              :id="keyNameInputId"
              v-model="form.name"
              :name="keyNameFieldName"
              required
              placeholder="例如：主 Key、备用 Key 1"
              maxlength="100"
              autocomplete="off"
              autocapitalize="none"
              autocorrect="off"
              spellcheck="false"
              data-form-type="other"
              data-lpignore="true"
              data-1p-ignore="true"
            />
          </div>
          <div>
            <Label for="rate_multiplier">成本倍率 *</Label>
            <Input
              id="rate_multiplier"
              v-model.number="form.rate_multiplier"
              type="number"
              step="0.01"
              min="0.01"
              required
              placeholder="1.0"
            />
            <p class="text-xs text-muted-foreground mt-1">
              真实成本 = 表面成本 × 倍率
            </p>
          </div>
        </div>

        <div>
          <Label :for="apiKeyInputId">API 密钥 {{ editingKey ? '' : '*' }}</Label>
          <Input
            :id="apiKeyInputId"
            v-model="form.api_key"
            :name="apiKeyFieldName"
            :type="apiKeyInputType"
            :required="!editingKey"
            :placeholder="editingKey ? editingKey.api_key_masked : 'sk-...'"
            :class="getApiKeyInputClass()"
            autocomplete="new-password"
            autocapitalize="none"
            autocorrect="off"
            spellcheck="false"
            data-form-type="other"
            data-lpignore="true"
            data-1p-ignore="true"
            @focus="apiKeyFocused = true"
            @blur="apiKeyFocused = form.api_key.trim().length > 0"
          />
          <p
            v-if="apiKeyError"
            class="text-xs text-destructive mt-1"
          >
            {{ apiKeyError }}
          </p>
          <p
            v-else-if="editingKey"
            class="text-xs text-muted-foreground mt-1"
          >
            留空表示不修改，输入新值则覆盖
          </p>
        </div>

        <div>
          <Label for="note">备注</Label>
          <Input
            id="note"
            v-model="form.note"
            placeholder="可选的备注信息"
          />
        </div>
      </div>

      <!-- 调度与限流 -->
      <div class="space-y-3">
        <h3 class="text-sm font-medium border-b pb-2">
          调度与限流
        </h3>
        <div class="grid grid-cols-2 gap-4">
          <div>
            <Label for="internal_priority">内部优先级</Label>
            <Input
              id="internal_priority"
              v-model.number="form.internal_priority"
              type="number"
              min="0"
            />
            <p class="text-xs text-muted-foreground mt-1">
              数字越小越优先
            </p>
          </div>
          <div>
            <Label for="max_concurrent">最大并发</Label>
            <Input
              id="max_concurrent"
              :model-value="form.max_concurrent ?? ''"
              type="number"
              min="1"
              placeholder="留空启用自适应"
              @update:model-value="(v) => form.max_concurrent = parseNumberInput(v)"
            />
            <p class="text-xs text-muted-foreground mt-1">
              留空 = 自适应模式
            </p>
          </div>
        </div>

        <div class="grid grid-cols-3 gap-4">
          <div>
            <Label for="rate_limit">速率限制(/分钟)</Label>
            <Input
              id="rate_limit"
              :model-value="form.rate_limit ?? ''"
              type="number"
              min="1"
              @update:model-value="(v) => form.rate_limit = parseNumberInput(v)"
            />
          </div>
          <div>
            <Label for="daily_limit">每日限制</Label>
            <Input
              id="daily_limit"
              :model-value="form.daily_limit ?? ''"
              type="number"
              min="1"
              @update:model-value="(v) => form.daily_limit = parseNumberInput(v)"
            />
          </div>
          <div>
            <Label for="monthly_limit">每月限制</Label>
            <Input
              id="monthly_limit"
              :model-value="form.monthly_limit ?? ''"
              type="number"
              min="1"
              @update:model-value="(v) => form.monthly_limit = parseNumberInput(v)"
            />
          </div>
        </div>
      </div>

      <!-- 缓存与熔断 -->
      <div class="space-y-3">
        <h3 class="text-sm font-medium border-b pb-2">
          缓存与熔断
        </h3>
        <div class="grid grid-cols-2 gap-4">
          <div>
            <Label for="cache_ttl_minutes">缓存 TTL (分钟)</Label>
            <Input
              id="cache_ttl_minutes"
              :model-value="form.cache_ttl_minutes ?? ''"
              type="number"
              min="0"
              max="60"
              @update:model-value="(v) => form.cache_ttl_minutes = parseNumberInput(v, { min: 0, max: 60 }) ?? 5"
            />
            <p class="text-xs text-muted-foreground mt-1">
              0 = 禁用缓存亲和性
            </p>
          </div>
          <div>
            <Label for="max_probe_interval_minutes">熔断探测间隔 (分钟)</Label>
            <Input
              id="max_probe_interval_minutes"
              :model-value="form.max_probe_interval_minutes ?? ''"
              type="number"
              min="2"
              max="32"
              placeholder="32"
              @update:model-value="(v) => form.max_probe_interval_minutes = parseNumberInput(v, { min: 2, max: 32 }) ?? 32"
            />
            <p class="text-xs text-muted-foreground mt-1">
              范围 2-32 分钟
            </p>
          </div>
        </div>
      </div>

      <!-- 能力标签配置 -->
      <div
        v-if="availableCapabilities.length > 0"
        class="space-y-3"
      >
        <h3 class="text-sm font-medium border-b pb-2">
          能力标签
        </h3>
        <div class="flex flex-wrap gap-2">
          <label
            v-for="cap in availableCapabilities"
            :key="cap.name"
            class="flex items-center gap-2 px-3 py-1.5 rounded-md border border-border bg-muted/30 cursor-pointer text-sm"
          >
            <input
              type="checkbox"
              :checked="form.capabilities[cap.name] || false"
              class="rounded"
              @change="form.capabilities[cap.name] = !form.capabilities[cap.name]"
            >
            <span>{{ cap.display_name }}</span>
          </label>
        </div>
      </div>
    </form>

    <template #footer>
      <Button
        variant="outline"
        @click="handleCancel"
      >
        取消
      </Button>
      <Button
        :disabled="saving"
        @click="handleSave"
      >
        {{ saving ? '保存中...' : '保存' }}
      </Button>
    </template>
  </Dialog>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { Dialog, Button, Input, Label } from '@/components/ui'
import { Key, SquarePen } from 'lucide-vue-next'
import { useToast } from '@/composables/useToast'
import { useFormDialog } from '@/composables/useFormDialog'
import { parseApiError } from '@/utils/errorParser'
import { parseNumberInput } from '@/utils/form'
import { log } from '@/utils/logger'
import {
  addEndpointKey,
  updateEndpointKey,
  addProviderKey,
  updateProviderKey,
  getAllCapabilities,
  type EndpointAPIKey,
  type EndpointAPIKeyUpdate,
  type ProviderEndpoint,
  type CapabilityDefinition
} from '@/api/endpoints'

const props = defineProps<{
  open: boolean
  endpoint: ProviderEndpoint | null
  editingKey: EndpointAPIKey | null
  providerId: string | null
}>()

const emit = defineEmits<{
  close: []
  saved: []
}>()

const { success, error: showError } = useToast()

const isOpen = computed(() => props.open)
const saving = ref(false)
const formNonce = ref(createFieldNonce())
const keyNameInputId = computed(() => `key-name-${formNonce.value}`)
const apiKeyInputId = computed(() => `api-key-${formNonce.value}`)
const keyNameFieldName = computed(() => `key-name-field-${formNonce.value}`)
const apiKeyFieldName = computed(() => `api-key-field-${formNonce.value}`)
const apiKeyFocused = ref(false)
const apiKeyInputType = computed(() =>
  apiKeyFocused.value || form.value.api_key.trim().length > 0 ? 'password' : 'text'
)

// 可用的能力列表
const availableCapabilities = ref<CapabilityDefinition[]>([])

const form = ref({
  name: '',
  api_key: '',
  rate_multiplier: 1.0,
  internal_priority: 50,
  max_concurrent: undefined as number | undefined,
  rate_limit: undefined as number | undefined,
  daily_limit: undefined as number | undefined,
  monthly_limit: undefined as number | undefined,
  cache_ttl_minutes: 5,
  max_probe_interval_minutes: 32,
  note: '',
  is_active: true,
  capabilities: {} as Record<string, boolean>
})

// 加载能力列表
async function loadCapabilities() {
  try {
    availableCapabilities.value = await getAllCapabilities()
  } catch (err) {
    log.error('Failed to load capabilities:', err)
  }
}

onMounted(() => {
  loadCapabilities()
})

// API 密钥输入框样式计算
function getApiKeyInputClass(): string {
  const classes = []
  if (apiKeyError.value) {
    classes.push('border-destructive')
  }
  if (!apiKeyFocused.value && !form.value.api_key) {
    classes.push('text-transparent caret-transparent selection:bg-transparent selection:text-transparent')
  }
  return classes.join(' ')
}


// API 密钥验证错误信息
const apiKeyError = computed(() => {
  const apiKey = form.value.api_key.trim()
  if (!apiKey) {
    // 新增模式下必填
    if (!props.editingKey) {
      return ''  // 空值由 required 属性处理
    }
    // 编辑模式下可以为空（表示不修改）
    return ''
  }

  // 如果输入了值，检查长度
  if (apiKey.length < 3) {
    return 'API 密钥至少需要 3 个字符'
  }

  return ''
})

// 重置表单
function resetForm() {
  formNonce.value = createFieldNonce()
  apiKeyFocused.value = false
  form.value = {
    name: '',
    api_key: '',
    rate_multiplier: 1.0,
    internal_priority: 50,
    max_concurrent: undefined,
    rate_limit: undefined,
    daily_limit: undefined,
    monthly_limit: undefined,
    cache_ttl_minutes: 5,
    max_probe_interval_minutes: 32,
    note: '',
    is_active: true,
    capabilities: {}
  }
}

// 加载密钥数据（编辑模式）
function loadKeyData() {
  if (!props.editingKey) return
  formNonce.value = createFieldNonce()
  apiKeyFocused.value = false
  form.value = {
    name: props.editingKey.name,
    api_key: '',
    rate_multiplier: props.editingKey.rate_multiplier || 1.0,
    internal_priority: props.editingKey.internal_priority ?? 50,
    // 保留原始的 null/undefined 状态，null 表示自适应模式
    max_concurrent: props.editingKey.max_concurrent ?? undefined,
    rate_limit: props.editingKey.rate_limit ?? undefined,
    daily_limit: props.editingKey.daily_limit ?? undefined,
    monthly_limit: props.editingKey.monthly_limit ?? undefined,
    cache_ttl_minutes: props.editingKey.cache_ttl_minutes ?? 5,
    max_probe_interval_minutes: props.editingKey.max_probe_interval_minutes ?? 32,
    note: props.editingKey.note || '',
    is_active: props.editingKey.is_active,
    capabilities: { ...(props.editingKey.capabilities || {}) }
  }
}

// 使用 useFormDialog 统一处理对话框逻辑
const { isEditMode, handleDialogUpdate, handleCancel } = useFormDialog({
  isOpen: () => props.open,
  entity: () => props.editingKey,
  isLoading: saving,
  onClose: () => emit('close'),
  loadData: loadKeyData,
  resetForm,
})

function createFieldNonce(): string {
  return Math.random().toString(36).slice(2, 10)
}

async function handleSave() {
  if (!props.endpoint && !props.providerId) return

  // 提交前验证
  if (apiKeyError.value) {
    showError(apiKeyError.value, '验证失败')
    return
  }

  // 新增模式下，API 密钥必填
  if (!props.editingKey && !form.value.api_key.trim()) {
    showError('请输入 API 密钥', '验证失败')
    return
  }

  // 过滤出有效的能力配置（只包含值为 true 的）
  const activeCapabilities: Record<string, boolean> = {}
  for (const [key, value] of Object.entries(form.value.capabilities)) {
    if (value) {
      activeCapabilities[key] = true
    }
  }
  const capabilitiesData = Object.keys(activeCapabilities).length > 0 ? activeCapabilities : null

  saving.value = true
  try {
    const commonData = {
      name: form.value.name,
      rate_multiplier: form.value.rate_multiplier,
      internal_priority: form.value.internal_priority,
      // 显式使用 null 表示自适应模式，这样后端能区分"未提供"和"设置为 null"
      // 注意：只有 max_concurrent 需要这种处理，因为它有"自适应模式"的概念
      // 其他限制字段（rate_limit 等）不支持"清空"操作，undefined 会被 JSON 忽略即不更新
      max_concurrent: form.value.max_concurrent === undefined ? null : form.value.max_concurrent,
      rate_limit: form.value.rate_limit,
      daily_limit: form.value.daily_limit,
      monthly_limit: form.value.monthly_limit,
      cache_ttl_minutes: form.value.cache_ttl_minutes,
      max_probe_interval_minutes: form.value.max_probe_interval_minutes,
      note: form.value.note,
      is_active: form.value.is_active,
      capabilities: capabilitiesData
    }

    if (props.editingKey) {
      // 更新模式
      const updateData: any = { ...commonData }

      if (form.value.api_key.trim()) {
        updateData.api_key = form.value.api_key
      }

      // Check if it's a shared key or endpoint key
      if (props.endpoint) {
        await updateEndpointKey(props.editingKey.id, updateData)
      } else {
        // Shared Key Update
        await updateProviderKey(props.editingKey.id, updateData)
      }
      success('密钥已更新', '成功')
    } else {
      // 新增模式
      if (props.endpoint) {
        // Add Endpoint Key
        await addEndpointKey(props.endpoint.id, {
          endpoint_id: props.endpoint.id,
          api_key: form.value.api_key,
          ...commonData,
          capabilities: capabilitiesData || undefined
        })
      } else if (props.providerId) {
        // Add Shared Key
        await addProviderKey(props.providerId, {
          api_key: form.value.api_key,
          ...commonData,
          capabilities: capabilitiesData || undefined
        })
      } else {
        throw new Error('Missing endpoint or provider context')
      }
      success('密钥已添加', '成功')
    }

    emit('saved')
    emit('close')
  } catch (err: any) {
    const errorMessage = parseApiError(err, '保存密钥失败')
    showError(errorMessage, '错误')
  } finally {
    saving.value = false
  }
}
</script>
