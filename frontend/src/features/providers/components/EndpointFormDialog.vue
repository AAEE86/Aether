<template>
  <Dialog
    :model-value="internalOpen"
    title="端点管理"
    :description="`管理 ${provider?.name} 的 API 端点`"
    :icon="Settings"
    size="2xl"
    @update:model-value="handleDialogUpdate"
  >
    <div class="flex flex-col gap-4">
      <!-- 已有端点列表（可滚动） -->
      <div
        v-if="localEndpoints.length > 0"
        class="space-y-3 max-h-[50vh] overflow-y-auto scrollbar-hide"
      >
        <Label class="text-muted-foreground">已配置的端点</Label>

        <!-- 端点卡片列表 -->
        <div class="space-y-3">
          <div
            v-for="endpoint in localEndpoints"
            :key="endpoint.id"
            class="rounded-lg border bg-card"
            :class="{ 'opacity-60': !endpoint.is_active }"
          >
            <!-- 卡片头部：格式名称 + 状态 + 操作 -->
            <div class="flex items-center justify-between px-4 py-2.5 bg-muted/30 border-b">
              <div class="flex items-center gap-3">
                <span class="font-medium">{{ API_FORMAT_LABELS[endpoint.api_format] || endpoint.api_format }}</span>
                <Badge
                  v-if="!endpoint.is_active"
                  variant="secondary"
                  class="text-xs"
                >
                  已停用
                </Badge>
              </div>
              <div class="flex items-center gap-1.5">
                <!-- 格式转换按钮 -->
                <span
                  class="mr-1"
                  :title="isEndpointFormatConversionDisabled ? formatConversionDisabledTooltip : (endpoint.format_acceptance_config?.enabled ? '已启用格式转换（点击关闭）' : '启用格式转换')"
                >
                  <Button
                    variant="ghost"
                    size="icon"
                    :class="`h-7 w-7 ${endpoint.format_acceptance_config?.enabled ? 'text-primary' : ''} ${isEndpointFormatConversionDisabled ? 'opacity-50' : ''}`"
                    :disabled="togglingFormatEndpointId === endpoint.id || isEndpointFormatConversionDisabled"
                    @click="handleToggleFormatConversion(endpoint)"
                  >
                    <Shuffle class="w-3.5 h-3.5" />
                  </Button>
                </span>
                <!-- 启用/停用 -->
                <Button
                  variant="ghost"
                  size="icon"
                  class="h-7 w-7"
                  :title="endpoint.is_active ? '停用' : '启用'"
                  :disabled="togglingEndpointId === endpoint.id"
                  @click="handleToggleEndpoint(endpoint)"
                >
                  <Power class="w-3.5 h-3.5" />
                </Button>
                <!-- 删除 -->
                <Button
                  v-if="!isFixedProvider"
                  variant="ghost"
                  size="icon"
                  class="h-7 w-7 hover:text-destructive"
                  title="删除"
                  :disabled="deletingEndpointId === endpoint.id"
                  @click="handleDeleteEndpoint(endpoint)"
                >
                  <Trash2 class="w-3.5 h-3.5" />
                </Button>
              </div>
            </div>

            <!-- 卡片内容 -->
            <div class="p-4 space-y-4">
              <!-- URL 配置区 -->
              <div class="flex items-end gap-3">
                <div class="flex-1 min-w-0 grid grid-cols-3 gap-3">
                  <div class="col-span-2 space-y-1.5">
                    <Label class="text-xs text-muted-foreground">Base URL</Label>
                    <Input
                      :model-value="getEndpointEditState(endpoint.id)?.url ?? endpoint.base_url"
                      :placeholder="provider?.website || 'https://api.example.com'"
                      :disabled="isFixedProvider"
                      @update:model-value="(v) => updateEndpointField(endpoint.id, 'url', v)"
                    />
                  </div>
                  <div class="space-y-1.5">
                    <Label class="text-xs text-muted-foreground">自定义路径</Label>
                    <Input
                      :model-value="getEndpointEditState(endpoint.id)?.path ?? (endpoint.custom_path || '')"
                      :placeholder="getDefaultPath(endpoint.api_format, endpoint.base_url) || '留空使用默认'"
                      :disabled="isFixedProvider"
                      @update:model-value="(v) => updateEndpointField(endpoint.id, 'path', v)"
                    />
                  </div>
                </div>
                <!-- 保存/撤销按钮（URL/路径有修改时显示） -->
                <div
                  v-if="!isFixedProvider && hasUrlChanges(endpoint)"
                  class="flex items-center gap-1 shrink-0"
                >
                  <Button
                    variant="ghost"
                    size="icon"
                    class="h-9 w-9"
                    title="保存"
                    :disabled="savingEndpointId === endpoint.id"
                    @click="saveEndpoint(endpoint)"
                  >
                    <Check class="w-4 h-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    class="h-9 w-9"
                    title="撤销"
                    @click="resetEndpointChanges(endpoint)"
                  >
                    <RotateCcw class="w-4 h-4" />
                  </Button>
                </div>
              </div>

              <!-- 请求规则（合并请求头和请求体规则） -->
              <Collapsible v-model:open="endpointRulesExpanded[endpoint.id]">
                <div class="flex items-center gap-2">
                  <!-- 有规则时显示可折叠的触发器 -->
                  <CollapsibleTrigger
                    v-if="getTotalRulesCount(endpoint) > 0"
                    as-child
                  >
                    <button
                      type="button"
                      class="flex items-center gap-2 py-1.5 px-2 -mx-2 rounded-md hover:bg-muted/50 transition-colors"
                    >
                      <ChevronRight
                        class="w-4 h-4 transition-transform text-muted-foreground"
                        :class="{ 'rotate-90': endpointRulesExpanded[endpoint.id] }"
                      />
                      <span class="text-sm font-medium">请求规则</span>
                      <Badge
                        variant="secondary"
                        class="text-xs"
                      >
                        {{ getTotalRulesCount(endpoint) }} 条
                      </Badge>
                    </button>
                  </CollapsibleTrigger>
                  <!-- 没有规则时只显示标题 -->
                  <span
                    v-else
                    class="text-sm text-muted-foreground py-1.5"
                  >
                    请求规则
                  </span>
                  <div class="flex-1" />
                  <div class="flex items-center gap-1 shrink-0">
                    <Button
                      v-if="hasRulesChanges(endpoint) || hasBodyRulesChanges(endpoint)"
                      variant="ghost"
                      size="icon"
                      class="h-7 w-7"
                      title="保存规则"
                      :disabled="savingEndpointId === endpoint.id"
                      @click="saveEndpoint(endpoint)"
                    >
                      <Check class="w-3.5 h-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      class="h-7 text-xs px-2"
                      title="添加请求头规则"
                      @click="handleAddEndpointRule(endpoint.id)"
                    >
                      <Plus class="w-3 h-3 mr-1" />
                      请求头
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      class="h-7 text-xs px-2"
                      title="添加请求体规则"
                      @click="handleAddEndpointBodyRule(endpoint.id)"
                    >
                      <Plus class="w-3 h-3 mr-1" />
                      请求体
                    </Button>
                  </div>
                </div>
                <CollapsibleContent class="pt-3">
                  <div class="space-y-2">
                    <!-- 请求头规则列表 - 主题色边框 -->
                    <div
                      v-for="(rule, index) in getEndpointEditRules(endpoint.id)"
                      :key="`header-${index}`"
                      class="flex items-center gap-1.5 px-2 py-1.5 rounded-md border-l-4 border-primary/60 bg-muted/30"
                    >
                      <span
                        class="text-[10px] font-semibold text-primary shrink-0"
                        title="请求头"
                      >H</span>
                      <Select
                        :model-value="rule.action"
                        :open="ruleSelectOpen[`${endpoint.id}-${index}`]"
                        @update:model-value="(v) => updateEndpointRuleAction(endpoint.id, index, v as 'set' | 'drop' | 'rename')"
                        @update:open="(v) => handleRuleSelectOpen(endpoint.id, index, v)"
                      >
                        <SelectTrigger class="w-[88px] h-7 text-xs shrink-0">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="set">
                            覆写
                          </SelectItem>
                          <SelectItem value="drop">
                            删除
                          </SelectItem>
                          <SelectItem value="rename">
                            重命名
                          </SelectItem>
                        </SelectContent>
                      </Select>
                      <template v-if="rule.action === 'set'">
                        <Input
                          :model-value="rule.key"
                          placeholder="名称"
                          size="sm"
                          class="flex-1 min-w-0 h-7 text-xs"
                          @update:model-value="(v) => updateEndpointRuleField(endpoint.id, index, 'key', v)"
                        />
                        <span class="text-muted-foreground text-xs">=</span>
                        <Input
                          :model-value="rule.value"
                          placeholder="值"
                          size="sm"
                          class="flex-1 min-w-0 h-7 text-xs"
                          @update:model-value="(v) => updateEndpointRuleField(endpoint.id, index, 'value', v)"
                        />
                      </template>
                      <template v-else-if="rule.action === 'drop'">
                        <Input
                          :model-value="rule.key"
                          placeholder="要删除的名称"
                          size="sm"
                          class="flex-1 min-w-0 h-7 text-xs"
                          @update:model-value="(v) => updateEndpointRuleField(endpoint.id, index, 'key', v)"
                        />
                      </template>
                      <template v-else-if="rule.action === 'rename'">
                        <Input
                          :model-value="rule.from"
                          placeholder="原名"
                          size="sm"
                          class="flex-1 min-w-0 h-7 text-xs"
                          @update:model-value="(v) => updateEndpointRuleField(endpoint.id, index, 'from', v)"
                        />
                        <span class="text-muted-foreground text-xs">→</span>
                        <Input
                          :model-value="rule.to"
                          placeholder="新名"
                          size="sm"
                          class="flex-1 min-w-0 h-7 text-xs"
                          @update:model-value="(v) => updateEndpointRuleField(endpoint.id, index, 'to', v)"
                        />
                      </template>
                      <Button
                        variant="ghost"
                        size="icon"
                        class="h-7 w-7 shrink-0"
                        @click="removeEndpointRule(endpoint.id, index)"
                      >
                        <X class="w-3 h-3" />
                      </Button>
                    </div>

                    <!-- 请求体规则列表 - 次要色边框 -->
                    <div
                      v-for="(rule, index) in getEndpointEditBodyRules(endpoint.id)"
                      :key="`body-${index}`"
                      class="flex items-center gap-1.5 px-2 py-1.5 rounded-md border-l-4 border-muted-foreground/40 bg-muted/30"
                    >
                      <span
                        class="text-[10px] font-semibold text-muted-foreground shrink-0"
                        title="请求体"
                      >B</span>
                      <Select
                        :model-value="rule.action"
                        :open="bodyRuleSelectOpen[`${endpoint.id}-${index}`]"
                        @update:model-value="(v) => updateEndpointBodyRuleAction(endpoint.id, index, v as 'set' | 'drop' | 'rename')"
                        @update:open="(v) => handleBodyRuleSelectOpen(endpoint.id, index, v)"
                      >
                        <SelectTrigger class="w-[88px] h-7 text-xs shrink-0">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="set">
                            覆写
                          </SelectItem>
                          <SelectItem value="drop">
                            删除
                          </SelectItem>
                          <SelectItem value="rename">
                            重命名
                          </SelectItem>
                        </SelectContent>
                      </Select>
                      <template v-if="rule.action === 'set'">
                        <Input
                          :model-value="rule.path"
                          placeholder="字段名"
                          size="sm"
                          class="flex-1 min-w-0 h-7 text-xs"
                          @update:model-value="(v) => updateEndpointBodyRuleField(endpoint.id, index, 'path', v)"
                        />
                        <span class="text-muted-foreground text-xs">=</span>
                        <Input
                          :model-value="rule.value"
                          placeholder="值"
                          size="sm"
                          class="flex-1 min-w-0 h-7 text-xs"
                          @update:model-value="(v) => updateEndpointBodyRuleField(endpoint.id, index, 'value', v)"
                        />
                      </template>
                      <template v-else-if="rule.action === 'drop'">
                        <Input
                          :model-value="rule.path"
                          placeholder="要删除的字段名"
                          size="sm"
                          class="flex-1 min-w-0 h-7 text-xs"
                          @update:model-value="(v) => updateEndpointBodyRuleField(endpoint.id, index, 'path', v)"
                        />
                      </template>
                      <template v-else-if="rule.action === 'rename'">
                        <Input
                          :model-value="rule.from"
                          placeholder="原名"
                          size="sm"
                          class="flex-1 min-w-0 h-7 text-xs"
                          @update:model-value="(v) => updateEndpointBodyRuleField(endpoint.id, index, 'from', v)"
                        />
                        <span class="text-muted-foreground text-xs">→</span>
                        <Input
                          :model-value="rule.to"
                          placeholder="新名"
                          size="sm"
                          class="flex-1 min-w-0 h-7 text-xs"
                          @update:model-value="(v) => updateEndpointBodyRuleField(endpoint.id, index, 'to', v)"
                        />
                      </template>
                      <Button
                        variant="ghost"
                        size="icon"
                        class="h-7 w-7 shrink-0"
                        @click="removeEndpointBodyRule(endpoint.id, index)"
                      >
                        <X class="w-3 h-3" />
                      </Button>
                    </div>
                  </div>
                </CollapsibleContent>
              </Collapsible>
            </div>
          </div>
        </div>
      </div>

      <!-- 添加新端点 -->
      <div
        v-if="!isFixedProvider && availableFormats.length > 0"
        class="rounded-lg border border-dashed p-3"
      >
        <!-- 卡片头部：API 格式选择 + 添加按钮 -->
        <div class="flex items-center justify-between px-4 py-2.5 bg-muted/30 border-b border-dashed">
          <Select
            v-model="newEndpoint.api_format"
            :open="formatSelectOpen"
            @update:open="handleFormatSelectOpen"
          >
            <SelectTrigger class="h-auto w-auto gap-1.5 !border-0 bg-transparent !shadow-none p-0 font-medium rounded-none flex-row-reverse !ring-0 !ring-offset-0 !outline-none [&>svg]:h-4 [&>svg]:w-4 [&>svg]:opacity-70">
              <SelectValue placeholder="选择格式..." />
            </SelectTrigger>
            <SelectContent>
              <SelectItem
                v-for="format in availableFormats"
                :key="format.value"
                :value="format.value"
              >
                {{ format.label }}
              </SelectItem>
            </SelectContent>
          </Select>
          <Button
            variant="ghost"
            size="icon"
            class="h-7 w-7 text-primary hover:text-primary"
            title="添加"
            :disabled="!newEndpoint.api_format || (!newEndpoint.base_url?.trim() && !provider?.website?.trim()) || addingEndpoint"
            @click="handleAddEndpoint"
          >
            <Plus class="w-3.5 h-3.5" />
          </Button>
        </div>
        <!-- 卡片内容：URL 配置 -->
        <div class="p-4">
          <div class="flex items-end gap-3">
            <div class="flex-1 min-w-0 grid grid-cols-3 gap-3">
              <div class="col-span-2 space-y-1.5">
                <Label class="text-xs text-muted-foreground">Base URL</Label>
                <Input
                  v-model="newEndpoint.base_url"
                  size="sm"
                  :placeholder="provider?.website || 'https://api.example.com'"
                />
              </div>
              <div class="space-y-1.5">
                <Label class="text-xs text-muted-foreground">自定义路径</Label>
                <Input
                  v-model="newEndpoint.custom_path"
                  size="sm"
                  :placeholder="newEndpointDefaultPath || '留空使用默认'"
                />
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- 空状态 -->
      <div
        v-if="localEndpoints.length === 0 && availableFormats.length === 0"
        class="text-center py-8 text-muted-foreground"
      >
        <p>所有 API 格式都已配置</p>
      </div>
    </div>

    <template #footer>
      <Button
        variant="outline"
        @click="handleClose"
      >
        关闭
      </Button>
    </template>
  </Dialog>

  <!-- 删除端点确认弹窗 -->
  <AlertDialog
    :model-value="deleteConfirmOpen"
    title="删除端点"
    :description="deleteConfirmDescription"
    confirm-text="删除"
    cancel-text="取消"
    type="danger"
    @update:model-value="deleteConfirmOpen = $event"
    @confirm="confirmDeleteEndpoint"
    @cancel="deleteConfirmOpen = false"
  />
</template>

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import {
  Dialog,
  Button,
  Input,
  Label,
  Badge,
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
  Collapsible,
  CollapsibleTrigger,
  CollapsibleContent,
} from '@/components/ui'
import { Settings, Trash2, Check, X, Power, ChevronRight, Plus, Shuffle, RotateCcw } from 'lucide-vue-next'
import { useToast } from '@/composables/useToast'
import { log } from '@/utils/logger'
import AlertDialog from '@/components/common/AlertDialog.vue'
import {
  createEndpoint,
  updateEndpoint,
  deleteEndpoint,
  API_FORMAT_LABELS,
  type ProviderEndpoint,
  type ProviderWithEndpointsSummary,
  type HeaderRule,
  type BodyRule,
} from '@/api/endpoints'
import { adminApi } from '@/api/admin'

// 编辑用的规则类型（统一的可编辑结构）
interface EditableRule {
  action: 'set' | 'drop' | 'rename'
  key: string      // set/drop 用
  value: string    // set 用
  from: string     // rename 用
  to: string       // rename 用
}

// 编辑用的请求体规则类型
interface EditableBodyRule {
  action: 'set' | 'drop' | 'rename'
  path: string     // set/drop 用
  value: string    // set 用
  from: string     // rename 用
  to: string       // rename 用
}

// 端点编辑状态（仅 URL、路径、规则，格式转换是直接保存的）
interface EndpointEditState {
  url: string
  path: string
  rules: EditableRule[]
  bodyRules: EditableBodyRule[]
}

const props = defineProps<{
  modelValue: boolean
  provider: ProviderWithEndpointsSummary | null
  endpoints?: ProviderEndpoint[]
  systemFormatConversionEnabled?: boolean
  providerFormatConversionEnabled?: boolean
}>()

const emit = defineEmits<{
  'update:modelValue': [value: boolean]
  'endpointCreated': []
  'endpointUpdated': []
}>()

// 计算端点级格式转换是否应该被禁用
const isEndpointFormatConversionDisabled = computed(() => {
  return props.systemFormatConversionEnabled || props.providerFormatConversionEnabled
})

// 获取禁用提示
const formatConversionDisabledTooltip = computed(() => {
  if (props.systemFormatConversionEnabled) {
    return '请先关闭系统级开关'
  }
  if (props.providerFormatConversionEnabled) {
    return '请先关闭提供商级开关'
  }
  return ''
})

const { success, error: showError } = useToast()

// 规则 Select 的展开状态（与 Collapsible 分开管理）
const ruleSelectOpen = ref<Record<string, boolean>>({})

// 打开规则选择器时关闭其他所有下拉
function handleRuleSelectOpen(endpointId: string, index: number, open: boolean) {
  if (open) {
    formatSelectOpen.value = false
    // 关闭其他 Select
    Object.keys(ruleSelectOpen.value).forEach(key => {
      ruleSelectOpen.value[key] = false
    })
  }
  ruleSelectOpen.value[`${endpointId}-${index}`] = open
}

// 打开格式选择器时关闭其他所有下拉
function handleFormatSelectOpen(open: boolean) {
  if (open) {
    // 关闭所有规则 Select
    Object.keys(ruleSelectOpen.value).forEach(key => {
      ruleSelectOpen.value[key] = false
    })
    Object.keys(bodyRuleSelectOpen.value).forEach(key => {
      bodyRuleSelectOpen.value[key] = false
    })
  }
  formatSelectOpen.value = open
}

// 打开请求体规则选择器时关闭其他所有下拉
function handleBodyRuleSelectOpen(endpointId: string, index: number, open: boolean) {
  if (open) {
    formatSelectOpen.value = false
    // 关闭所有 Select
    Object.keys(ruleSelectOpen.value).forEach(key => {
      ruleSelectOpen.value[key] = false
    })
    Object.keys(bodyRuleSelectOpen.value).forEach(key => {
      bodyRuleSelectOpen.value[key] = false
    })
  }
  bodyRuleSelectOpen.value[`${endpointId}-${index}`] = open
}

// 状态
const addingEndpoint = ref(false)
const savingEndpointId = ref<string | null>(null)
const deletingEndpointId = ref<string | null>(null)
const togglingEndpointId = ref<string | null>(null)
const togglingFormatEndpointId = ref<string | null>(null)
const formatSelectOpen = ref(false)

// 删除确认弹窗状态
const deleteConfirmOpen = ref(false)
const endpointToDelete = ref<ProviderEndpoint | null>(null)

// 请求规则折叠状态
const endpointRulesExpanded = ref<Record<string, boolean>>({})

// 请求体规则 Select 的展开状态
const bodyRuleSelectOpen = ref<Record<string, boolean>>({})

// 每个端点的编辑状态（内联编辑）
const endpointEditStates = ref<Record<string, EndpointEditState>>({})

// 系统保留的 header 名称（不允许用户设置）
const RESERVED_HEADERS = new Set([
  'authorization',
  'x-api-key',
  'x-goog-api-key',
  'content-type',
  'content-length',
  'host',
])

// 系统保留的 body 字段名（不允许用户设置）
const RESERVED_BODY_FIELDS = new Set([
  'model',
  'stream',
])

// 内部状态
const internalOpen = computed(() => props.modelValue)

const isFixedProvider = computed(() => {
  const t = props.provider?.provider_type
  return !!t && t !== 'custom'
})

// 新端点表单
const newEndpoint = ref({
  api_format: '',
  base_url: '',
  custom_path: '',
})

// API 格式列表
const apiFormats = ref<Array<{ value: string; label: string; default_path: string }>>([])

// 本地端点列表
const localEndpoints = ref<ProviderEndpoint[]>([])

// 可用的格式（未添加的）
const availableFormats = computed(() => {
  const existingFormats = localEndpoints.value.map(e => e.api_format)
  return apiFormats.value.filter(f => !existingFormats.includes(f.value))
})

// 删除确认弹窗描述
const deleteConfirmDescription = computed(() => {
  if (!endpointToDelete.value) return ''
  const formatLabel = API_FORMAT_LABELS[endpointToDelete.value.api_format] || endpointToDelete.value.api_format
  return `确定要删除 ${formatLabel} 端点吗？关联密钥将移除对该 API 格式的支持。`
})

// 获取指定 API 格式的默认路径
function getDefaultPath(apiFormat: string, baseUrl?: string): string {
  const format = apiFormats.value.find(f => f.value === apiFormat)
  const defaultPath = format?.default_path || ''
  // Codex 端点使用 /responses 而非 /v1/responses
  if (apiFormat === 'openai:cli' && baseUrl && isCodexUrl(baseUrl)) {
    return '/responses'
  }
  return defaultPath
}

// 判断是否是 Codex OAuth 端点
function isCodexUrl(baseUrl: string): boolean {
  const url = baseUrl.replace(/\/+$/, '')
  return url.includes('/backend-api/codex') || url.endsWith('/codex')
}

// 初始化端点的编辑状态
function initEndpointEditState(endpoint: ProviderEndpoint): EndpointEditState {
  const rules: EditableRule[] = []
  if (endpoint.header_rules && endpoint.header_rules.length > 0) {
    for (const rule of endpoint.header_rules) {
      if (rule.action === 'set') {
        rules.push({ action: 'set', key: rule.key, value: rule.value || '', from: '', to: '' })
      } else if (rule.action === 'drop') {
        rules.push({ action: 'drop', key: rule.key, value: '', from: '', to: '' })
      } else if (rule.action === 'rename') {
        rules.push({ action: 'rename', key: '', value: '', from: rule.from, to: rule.to })
      }
    }
  }

  const bodyRules: EditableBodyRule[] = []
  if (endpoint.body_rules && endpoint.body_rules.length > 0) {
    for (const rule of endpoint.body_rules) {
      if (rule.action === 'set') {
        bodyRules.push({ action: 'set', path: rule.path, value: rule.value || '', from: '', to: '' })
      } else if (rule.action === 'drop') {
        bodyRules.push({ action: 'drop', path: rule.path, value: '', from: '', to: '' })
      } else if (rule.action === 'rename') {
        bodyRules.push({ action: 'rename', path: '', value: '', from: rule.from, to: rule.to })
      }
    }
  }

  return {
    url: endpoint.base_url,
    path: endpoint.custom_path || '',
    rules,
    bodyRules,
  }
}

// 获取端点的编辑状态
function getEndpointEditState(endpointId: string): EndpointEditState | undefined {
  return endpointEditStates.value[endpointId]
}

// 更新端点字段
function updateEndpointField(endpointId: string, field: 'url' | 'path', value: string) {
  if (!endpointEditStates.value[endpointId]) {
    const endpoint = localEndpoints.value.find(e => e.id === endpointId)
    if (endpoint) {
      endpointEditStates.value[endpointId] = initEndpointEditState(endpoint)
    }
  }
  if (endpointEditStates.value[endpointId]) {
    endpointEditStates.value[endpointId][field] = value
  }
}

// 获取端点的编辑规则
function getEndpointEditRules(endpointId: string): EditableRule[] {
  const state = endpointEditStates.value[endpointId]
  if (state) {
    return state.rules
  }
  // 从原始端点加载
  const endpoint = localEndpoints.value.find(e => e.id === endpointId)
  if (endpoint) {
    const newState = initEndpointEditState(endpoint)
    endpointEditStates.value[endpointId] = newState
    return newState.rules
  }
  return []
}

// 添加规则（同时自动展开折叠）
function handleAddEndpointRule(endpointId: string) {
  const rules = getEndpointEditRules(endpointId)
  rules.push({ action: 'set', key: '', value: '', from: '', to: '' })
  // 自动展开折叠
  endpointRulesExpanded.value[endpointId] = true
}

// 删除规则
function removeEndpointRule(endpointId: string, index: number) {
  const rules = getEndpointEditRules(endpointId)
  rules.splice(index, 1)
}

// 更新规则类型
function updateEndpointRuleAction(endpointId: string, index: number, action: 'set' | 'drop' | 'rename') {
  const rules = getEndpointEditRules(endpointId)
  if (rules[index]) {
    rules[index].action = action
    rules[index].key = ''
    rules[index].value = ''
    rules[index].from = ''
    rules[index].to = ''
  }
}

// 更新规则字段
function updateEndpointRuleField(endpointId: string, index: number, field: 'key' | 'value' | 'from' | 'to', value: string) {
  const rules = getEndpointEditRules(endpointId)
  if (rules[index]) {
    rules[index][field] = value
  }
}

// 验证规则 key（针对特定端点）
function validateRuleKeyForEndpoint(endpointId: string, key: string, index: number): string | null {
  const trimmedKey = key.trim().toLowerCase()
  if (!trimmedKey) return null

  if (RESERVED_HEADERS.has(trimmedKey)) {
    return `"${key}" 是系统保留的请求头`
  }

  const rules = getEndpointEditRules(endpointId)
  const duplicate = rules.findIndex(
    (r, i) => i !== index && (
      ((r.action === 'set' || r.action === 'drop') && r.key.trim().toLowerCase() === trimmedKey) ||
      (r.action === 'rename' && r.to.trim().toLowerCase() === trimmedKey)
    )
  )
  if (duplicate >= 0) {
    return '请求头名称重复'
  }

  return null
}

// 验证 rename from
function validateRenameFromForEndpoint(endpointId: string, from: string, index: number): string | null {
  const trimmedFrom = from.trim().toLowerCase()
  if (!trimmedFrom) return null

  const rules = getEndpointEditRules(endpointId)
  const duplicate = rules.findIndex(
    (r, i) => i !== index &&
      ((r.action === 'set' && r.key.trim().toLowerCase() === trimmedFrom) ||
       (r.action === 'drop' && r.key.trim().toLowerCase() === trimmedFrom) ||
       (r.action === 'rename' && r.from.trim().toLowerCase() === trimmedFrom))
  )
  if (duplicate >= 0) {
    return '该请求头已被其他规则处理'
  }

  return null
}

// 验证 rename to
function validateRenameToForEndpoint(endpointId: string, to: string, index: number): string | null {
  const trimmedTo = to.trim().toLowerCase()
  if (!trimmedTo) return null

  if (RESERVED_HEADERS.has(trimmedTo)) {
    return `"${to}" 是系统保留的请求头`
  }

  const rules = getEndpointEditRules(endpointId)
  const duplicate = rules.findIndex(
    (r, i) => i !== index &&
      ((r.action === 'set' && r.key.trim().toLowerCase() === trimmedTo) ||
       (r.action === 'rename' && r.to.trim().toLowerCase() === trimmedTo))
  )
  if (duplicate >= 0) {
    return '请求头名称重复'
  }

  return null
}

// 获取端点的请求头规则数量（有效的规则）
function getEndpointRulesCount(endpoint: ProviderEndpoint): number {
  const state = endpointEditStates.value[endpoint.id]
  if (state) {
    return state.rules.filter(r => {
      if (r.action === 'set' || r.action === 'drop') return r.key.trim()
      if (r.action === 'rename') return r.from.trim() && r.to.trim()
      return false
    }).length
  }
  return endpoint.header_rules?.length || 0
}

// 检查端点是否有任何规则（包括正在编辑的空规则）
function hasAnyRules(endpoint: ProviderEndpoint): boolean {
  const state = endpointEditStates.value[endpoint.id]
  if (state) {
    return state.rules.length > 0
  }
  return (endpoint.header_rules?.length || 0) > 0
}

// ========== 请求体规则相关函数 ==========

// 获取端点的编辑请求体规则
function getEndpointEditBodyRules(endpointId: string): EditableBodyRule[] {
  const state = endpointEditStates.value[endpointId]
  if (state) {
    return state.bodyRules
  }
  // 从原始端点加载
  const endpoint = localEndpoints.value.find(e => e.id === endpointId)
  if (endpoint) {
    const newState = initEndpointEditState(endpoint)
    endpointEditStates.value[endpointId] = newState
    return newState.bodyRules
  }
  return []
}

// 添加请求体规则（同时自动展开折叠）
function handleAddEndpointBodyRule(endpointId: string) {
  const rules = getEndpointEditBodyRules(endpointId)
  rules.push({ action: 'set', path: '', value: '', from: '', to: '' })
  // 自动展开折叠
  endpointRulesExpanded.value[endpointId] = true
}

// 删除请求体规则
function removeEndpointBodyRule(endpointId: string, index: number) {
  const rules = getEndpointEditBodyRules(endpointId)
  rules.splice(index, 1)
}

// 更新请求体规则类型
function updateEndpointBodyRuleAction(endpointId: string, index: number, action: 'set' | 'drop' | 'rename') {
  const rules = getEndpointEditBodyRules(endpointId)
  if (rules[index]) {
    rules[index].action = action
    rules[index].path = ''
    rules[index].value = ''
    rules[index].from = ''
    rules[index].to = ''
  }
}

// 更新请求体规则字段
function updateEndpointBodyRuleField(endpointId: string, index: number, field: 'path' | 'value' | 'from' | 'to', value: string) {
  const rules = getEndpointEditBodyRules(endpointId)
  if (rules[index]) {
    rules[index][field] = value
  }
}

// 验证请求体规则 path（针对特定端点）
function validateBodyRulePathForEndpoint(endpointId: string, path: string, index: number): string | null {
  const trimmedPath = path.trim().toLowerCase()
  if (!trimmedPath) return null

  if (RESERVED_BODY_FIELDS.has(trimmedPath)) {
    return `"${path}" 是系统保留的字段`
  }

  const rules = getEndpointEditBodyRules(endpointId)
  const duplicate = rules.findIndex(
    (r, i) => i !== index && (
      ((r.action === 'set' || r.action === 'drop') && r.path.trim().toLowerCase() === trimmedPath) ||
      (r.action === 'rename' && r.to.trim().toLowerCase() === trimmedPath)
    )
  )
  if (duplicate >= 0) {
    return '字段名重复'
  }

  return null
}

// 验证请求体 rename from
function validateBodyRenameFromForEndpoint(endpointId: string, from: string, index: number): string | null {
  const trimmedFrom = from.trim().toLowerCase()
  if (!trimmedFrom) return null

  const rules = getEndpointEditBodyRules(endpointId)
  const duplicate = rules.findIndex(
    (r, i) => i !== index &&
      ((r.action === 'set' && r.path.trim().toLowerCase() === trimmedFrom) ||
       (r.action === 'drop' && r.path.trim().toLowerCase() === trimmedFrom) ||
       (r.action === 'rename' && r.from.trim().toLowerCase() === trimmedFrom))
  )
  if (duplicate >= 0) {
    return '该字段已被其他规则处理'
  }

  return null
}

// 验证请求体 rename to
function validateBodyRenameToForEndpoint(endpointId: string, to: string, index: number): string | null {
  const trimmedTo = to.trim().toLowerCase()
  if (!trimmedTo) return null

  if (RESERVED_BODY_FIELDS.has(trimmedTo)) {
    return `"${to}" 是系统保留的字段`
  }

  const rules = getEndpointEditBodyRules(endpointId)
  const duplicate = rules.findIndex(
    (r, i) => i !== index &&
      ((r.action === 'set' && r.path.trim().toLowerCase() === trimmedTo) ||
       (r.action === 'rename' && r.to.trim().toLowerCase() === trimmedTo))
  )
  if (duplicate >= 0) {
    return '字段名重复'
  }

  return null
}

// 获取端点的请求体规则数量（有效的规则）
function getEndpointBodyRulesCount(endpoint: ProviderEndpoint): number {
  const state = endpointEditStates.value[endpoint.id]
  if (state) {
    return state.bodyRules.filter(r => {
      if (r.action === 'set' || r.action === 'drop') return r.path.trim()
      if (r.action === 'rename') return r.from.trim() && r.to.trim()
      return false
    }).length
  }
  return endpoint.body_rules?.length || 0
}

// 检查端点是否有任何请求体规则（包括正在编辑的空规则）
function hasAnyBodyRules(endpoint: ProviderEndpoint): boolean {
  const state = endpointEditStates.value[endpoint.id]
  if (state) {
    return state.bodyRules.length > 0
  }
  return (endpoint.body_rules?.length || 0) > 0
}

// 获取端点的总规则数量（请求头 + 请求体）
function getTotalRulesCount(endpoint: ProviderEndpoint): number {
  return getEndpointRulesCount(endpoint) + getEndpointBodyRulesCount(endpoint)
}

// 格式化请求头规则的显示标签
function formatHeaderRuleLabel(rule: EditableRule): string {
  if (rule.action === 'set') {
    if (!rule.key) return '(未设置)'
    return `${rule.key}=${rule.value || '...'}`
  } else if (rule.action === 'drop') {
    if (!rule.key) return '(未设置)'
    return `-${rule.key}`
  } else if (rule.action === 'rename') {
    if (!rule.from || !rule.to) return '(未设置)'
    return `${rule.from}→${rule.to}`
  }
  return '(未知)'
}

// 格式化请求体规则的显示标签
function formatBodyRuleLabel(rule: EditableBodyRule): string {
  if (rule.action === 'set') {
    if (!rule.path) return '(未设置)'
    return `${rule.path}=${rule.value || '...'}`
  } else if (rule.action === 'drop') {
    if (!rule.path) return '(未设置)'
    return `-${rule.path}`
  } else if (rule.action === 'rename') {
    if (!rule.from || !rule.to) return '(未设置)'
    return `${rule.from}→${rule.to}`
  }
  return '(未知)'
}

// 检查端点请求体规则是否有修改
function hasBodyRulesChanges(endpoint: ProviderEndpoint): boolean {
  const state = endpointEditStates.value[endpoint.id]
  if (!state) return false

  const originalRules = endpoint.body_rules || []
  const editedRules = state.bodyRules.filter(r => {
    if (r.action === 'set' || r.action === 'drop') return r.path.trim()
    if (r.action === 'rename') return r.from.trim() && r.to.trim()
    return false
  })
  if (editedRules.length !== originalRules.length) return true
  for (let i = 0; i < editedRules.length; i++) {
    const edited = editedRules[i]
    const original = originalRules[i]
    if (!original) return true
    if (edited.action !== original.action) return true
    if (edited.action === 'set' && original.action === 'set') {
      if (edited.path !== original.path || edited.value !== (original.value || '')) return true
    } else if (edited.action === 'drop' && original.action === 'drop') {
      if (edited.path !== original.path) return true
    } else if (edited.action === 'rename' && original.action === 'rename') {
      if (edited.from !== original.from || edited.to !== original.to) return true
    }
  }
  return false
}

// 将可编辑请求体规则数组转换为 API 需要的 BodyRule[]
function rulesToBodyRules(rules: EditableBodyRule[]): BodyRule[] | null {
  const result: BodyRule[] = []

  for (const rule of rules) {
    if (rule.action === 'set' && rule.path.trim()) {
      result.push({ action: 'set', path: rule.path.trim(), value: rule.value })
    } else if (rule.action === 'drop' && rule.path.trim()) {
      result.push({ action: 'drop', path: rule.path.trim() })
    } else if (rule.action === 'rename' && rule.from.trim() && rule.to.trim()) {
      result.push({ action: 'rename', from: rule.from.trim(), to: rule.to.trim() })
    }
  }

  return result.length > 0 ? result : null
}

// 检查请求体规则是否有验证错误
function hasBodyValidationErrorsForEndpoint(endpointId: string): boolean {
  const rules = getEndpointEditBodyRules(endpointId)
  for (let i = 0; i < rules.length; i++) {
    const rule = rules[i]
    if (rule.action === 'set' || rule.action === 'drop') {
      if (validateBodyRulePathForEndpoint(endpointId, rule.path, i)) return true
    } else if (rule.action === 'rename') {
      if (validateBodyRenameFromForEndpoint(endpointId, rule.from, i)) return true
      if (validateBodyRenameToForEndpoint(endpointId, rule.to, i)) return true
    }
  }
  return false
}

// 检查端点 URL/路径是否有修改
function hasUrlChanges(endpoint: ProviderEndpoint): boolean {
  const state = endpointEditStates.value[endpoint.id]
  if (!state) return false
  if (state.url !== endpoint.base_url) return true
  if (state.path !== (endpoint.custom_path || '')) return true
  return false
}

// 检查端点规则是否有修改
function hasRulesChanges(endpoint: ProviderEndpoint): boolean {
  const state = endpointEditStates.value[endpoint.id]
  if (!state) return false

  const originalRules = endpoint.header_rules || []
  const editedRules = state.rules.filter(r => {
    if (r.action === 'set' || r.action === 'drop') return r.key.trim()
    if (r.action === 'rename') return r.from.trim() && r.to.trim()
    return false
  })
  if (editedRules.length !== originalRules.length) return true
  for (let i = 0; i < editedRules.length; i++) {
    const edited = editedRules[i]
    const original = originalRules[i]
    if (!original) return true
    if (edited.action !== original.action) return true
    if (edited.action === 'set' && original.action === 'set') {
      if (edited.key !== original.key || edited.value !== (original.value || '')) return true
    } else if (edited.action === 'drop' && original.action === 'drop') {
      if (edited.key !== original.key) return true
    } else if (edited.action === 'rename' && original.action === 'rename') {
      if (edited.from !== original.from || edited.to !== original.to) return true
    }
  }
  return false
}

// 检查端点是否有修改（URL、路径或规则）
// 注：当前模板直接使用各子函数，此聚合函数保留供未来使用
function _hasEndpointChanges(endpoint: ProviderEndpoint): boolean {
  return hasUrlChanges(endpoint) || hasRulesChanges(endpoint) || hasBodyRulesChanges(endpoint)
}

// 重置端点修改
function resetEndpointChanges(endpoint: ProviderEndpoint) {
  endpointEditStates.value[endpoint.id] = initEndpointEditState(endpoint)
}

// 将可编辑规则数组转换为 API 需要的 HeaderRule[]
function rulesToHeaderRules(rules: EditableRule[]): HeaderRule[] | null {
  const result: HeaderRule[] = []

  for (const rule of rules) {
    if (rule.action === 'set' && rule.key.trim()) {
      result.push({ action: 'set', key: rule.key.trim(), value: rule.value })
    } else if (rule.action === 'drop' && rule.key.trim()) {
      result.push({ action: 'drop', key: rule.key.trim() })
    } else if (rule.action === 'rename' && rule.from.trim() && rule.to.trim()) {
      result.push({ action: 'rename', from: rule.from.trim(), to: rule.to.trim() })
    }
  }

  return result.length > 0 ? result : null
}

// 检查规则是否有验证错误
function hasValidationErrorsForEndpoint(endpointId: string): boolean {
  const rules = getEndpointEditRules(endpointId)
  for (let i = 0; i < rules.length; i++) {
    const rule = rules[i]
    if (rule.action === 'set' || rule.action === 'drop') {
      if (validateRuleKeyForEndpoint(endpointId, rule.key, i)) return true
    } else if (rule.action === 'rename') {
      if (validateRenameFromForEndpoint(endpointId, rule.from, i)) return true
      if (validateRenameToForEndpoint(endpointId, rule.to, i)) return true
    }
  }
  return false
}

// 新端点选择的格式的默认路径
const newEndpointDefaultPath = computed(() => {
  // 使用填写的 base_url 或 provider 的 website 来判断是否是 Codex 端点
  const baseUrl = newEndpoint.value.base_url || props.provider?.website || ''
  return getDefaultPath(newEndpoint.value.api_format, baseUrl)
})

// 加载 API 格式列表
const loadApiFormats = async () => {
  try {
    const response = await adminApi.getApiFormats()
    apiFormats.value = response.formats
  } catch (error) {
    log.error('加载API格式失败:', error)
  }
}

onMounted(() => {
  loadApiFormats()
})

// 监听 props 变化
watch(() => props.modelValue, (open) => {
  if (open) {
    localEndpoints.value = [...(props.endpoints || [])]
    // 清空编辑状态，重新从端点加载
    endpointEditStates.value = {}
    endpointRulesExpanded.value = {}
    // 初始化每个端点的编辑状态，有规则时默认展开
    for (const endpoint of localEndpoints.value) {
      endpointEditStates.value[endpoint.id] = initEndpointEditState(endpoint)
      // 有规则时默认展开
      const hasRules = (endpoint.header_rules?.length || 0) + (endpoint.body_rules?.length || 0) > 0
      endpointRulesExpanded.value[endpoint.id] = hasRules
    }
  } else {
    // 关闭对话框时完全清空新端点表单
    newEndpoint.value = { api_format: '', base_url: '', custom_path: '' }
  }
}, { immediate: true })

watch(() => props.endpoints, (endpoints) => {
  if (props.modelValue) {
    localEndpoints.value = [...(endpoints || [])]
    // 初始化新添加端点的编辑状态
    for (const endpoint of localEndpoints.value) {
      if (!endpointEditStates.value[endpoint.id]) {
        endpointEditStates.value[endpoint.id] = initEndpointEditState(endpoint)
      }
    }
  }
}, { deep: true })

// 保存端点
async function saveEndpoint(endpoint: ProviderEndpoint) {
  const state = endpointEditStates.value[endpoint.id]
  if (!state || !state.url) return

  // 检查规则是否有验证错误
  if (hasValidationErrorsForEndpoint(endpoint.id)) {
    showError('请修正请求头规则中的错误')
    return
  }

  // 检查请求体规则是否有验证错误
  if (hasBodyValidationErrorsForEndpoint(endpoint.id)) {
    showError('请修正请求体规则中的错误')
    return
  }

  savingEndpointId.value = endpoint.id
  try {
    await updateEndpoint(endpoint.id, {
      base_url: state.url,
      custom_path: state.path || null,
      header_rules: rulesToHeaderRules(state.rules),
      body_rules: rulesToBodyRules(state.bodyRules),
    })
    success('端点已更新')
    emit('endpointUpdated')
  } catch (error: any) {
    showError(error.response?.data?.detail || '更新失败', '错误')
  } finally {
    savingEndpointId.value = null
  }
}

// 切换格式转换（直接保存）
async function handleToggleFormatConversion(endpoint: ProviderEndpoint) {
  const currentEnabled = endpoint.format_acceptance_config?.enabled || false
  const newEnabled = !currentEnabled

  togglingFormatEndpointId.value = endpoint.id
  try {
    await updateEndpoint(endpoint.id, {
      format_acceptance_config: newEnabled ? { enabled: true } : null,
    })
    success(newEnabled ? '已启用格式转换' : '已关闭格式转换')
    emit('endpointUpdated')
  } catch (error: any) {
    showError(error.response?.data?.detail || '操作失败', '错误')
  } finally {
    togglingFormatEndpointId.value = null
  }
}

// 添加端点
async function handleAddEndpoint() {
  if (!props.provider || !newEndpoint.value.api_format) return

  // 如果没有输入 base_url，使用提供商的 website 作为默认值
  const baseUrl = newEndpoint.value.base_url || props.provider.website
  if (!baseUrl) {
    showError('请输入 Base URL')
    return
  }

  addingEndpoint.value = true
  try {
    await createEndpoint(props.provider.id, {
      provider_id: props.provider.id,
      api_format: newEndpoint.value.api_format,
      base_url: baseUrl,
      custom_path: newEndpoint.value.custom_path || undefined,
      is_active: true,
    })
    success(`已添加 ${API_FORMAT_LABELS[newEndpoint.value.api_format] || newEndpoint.value.api_format} 端点`)
    // 重置表单，保留 URL
    newEndpoint.value = { api_format: '', base_url: baseUrl, custom_path: '' }
    emit('endpointCreated')
  } catch (error: any) {
    showError(error.response?.data?.detail || '添加失败', '错误')
  } finally {
    addingEndpoint.value = false
  }
}

// 切换端点启用状态
async function handleToggleEndpoint(endpoint: ProviderEndpoint) {
  togglingEndpointId.value = endpoint.id
  try {
    const newStatus = !endpoint.is_active
    await updateEndpoint(endpoint.id, { is_active: newStatus })
    success(newStatus ? '端点已启用' : '端点已停用')
    emit('endpointUpdated')
  } catch (error: any) {
    showError(error.response?.data?.detail || '操作失败', '错误')
  } finally {
    togglingEndpointId.value = null
  }
}

// 删除端点 - 打开确认弹窗
function handleDeleteEndpoint(endpoint: ProviderEndpoint) {
  endpointToDelete.value = endpoint
  deleteConfirmOpen.value = true
}

// 确认删除端点
async function confirmDeleteEndpoint() {
  if (!endpointToDelete.value) return

  const endpoint = endpointToDelete.value
  deleteConfirmOpen.value = false
  deletingEndpointId.value = endpoint.id

  try {
    await deleteEndpoint(endpoint.id)
    success(`已删除 ${API_FORMAT_LABELS[endpoint.api_format] || endpoint.api_format} 端点`)
    emit('endpointUpdated')
  } catch (error: any) {
    showError(error.response?.data?.detail || '删除失败', '错误')
  } finally {
    deletingEndpointId.value = null
    endpointToDelete.value = null
  }
}

// 关闭对话框
function handleDialogUpdate(value: boolean) {
  emit('update:modelValue', value)
}

function handleClose() {
  emit('update:modelValue', false)
}
</script>
