<template>
  <Dialog
    :model-value="open"
    :title="dialogTitle"
    :description="dialogDescription"
    :icon="dialogIcon"
    size="md"
    @update:model-value="handleDialogUpdate"
  >
    <form
      class="space-y-4"
      @submit.prevent="handleSubmit"
    >
      <!-- 模式选择（仅创建时显示） -->
      <div
        v-if="!isEditMode"
        class="space-y-2"
      >
        <Label>创建类型 *</Label>
        <div class="grid grid-cols-2 gap-3">
          <button
            type="button"
            class="p-3 rounded-lg border-2 text-left transition-all"
            :class="[
              form.mapping_type === 'alias'
                ? 'border-primary bg-primary/5'
                : 'border-border hover:border-primary/50'
            ]"
            @click="form.mapping_type = 'alias'"
          >
            <div class="font-medium text-sm">
              别名
            </div>
            <div class="text-xs text-muted-foreground mt-1">
              名称简写，按目标模型计费
            </div>
          </button>
          <button
            type="button"
            class="p-3 rounded-lg border-2 text-left transition-all"
            :class="[
              form.mapping_type === 'mapping'
                ? 'border-primary bg-primary/5'
                : 'border-border hover:border-primary/50'
            ]"
            @click="form.mapping_type = 'mapping'"
          >
            <div class="font-medium text-sm">
              映射
            </div>
            <div class="text-xs text-muted-foreground mt-1">
              模型降级，按源模型计费
            </div>
          </button>
        </div>
      </div>

      <!-- 模式说明 -->
      <div class="rounded-lg border border-border bg-muted/50 p-3 text-sm">
        <p class="text-foreground font-medium mb-1">
          {{ form.mapping_type === 'alias' ? '别名模式' : '映射模式' }}
        </p>
        <p class="text-muted-foreground text-xs">
          {{ form.mapping_type === 'alias'
            ? '用户请求此别名时，会路由到目标模型，并按目标模型价格计费。'
            : '将源模型的请求转发到目标模型处理，按源模型价格计费。' }}
        </p>
      </div>

      <!-- Provider 选择/作用范围 -->
      <div
        v-if="showProviderSelect"
        class="space-y-2"
      >
        <Label>作用范围</Label>
        <!-- 固定 Provider 时显示只读 -->
        <div
          v-if="fixedProvider"
          class="px-3 py-2 border rounded-md bg-muted/50 text-sm"
        >
          仅 {{ fixedProvider.display_name || fixedProvider.name }}
        </div>
        <!-- 否则显示可选择的下拉 -->
        <Select
          v-else
          v-model:open="providerSelectOpen"
          :model-value="form.provider_id || 'global'"
          @update:model-value="handleProviderChange"
        >
          <SelectTrigger class="w-full">
            <SelectValue placeholder="选择作用范围" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="global">
              全局（所有 Provider）
            </SelectItem>
            <SelectItem
              v-for="p in providers"
              :key="p.id"
              :value="p.id"
            >
              仅 {{ p.display_name || p.name }}
            </SelectItem>
          </SelectContent>
        </Select>
      </div>

      <!-- 别名模式：别名名称 -->
      <div
        v-if="form.mapping_type === 'alias'"
        class="space-y-2"
      >
        <Label for="alias-name">别名名称 *</Label>
        <Input
          id="alias-name"
          v-model="form.alias"
          placeholder="如：sonnet, opus"
          :disabled="isEditMode"
          required
        />
        <p class="text-xs text-muted-foreground">
          {{ isEditMode ? '创建后不可修改' : '用户将使用此名称请求模型' }}
        </p>
      </div>

      <!-- 映射模式：选择源模型 -->
      <div
        v-else
        class="space-y-2"
      >
        <Label>源模型 (用户请求的模型) *</Label>
        <Select
          v-model:open="sourceModelSelectOpen"
          :model-value="form.alias"
          :disabled="isEditMode"
          @update:model-value="form.alias = $event"
        >
          <SelectTrigger
            class="w-full"
            :class="{ 'opacity-50': isEditMode }"
          >
            <SelectValue placeholder="请选择源模型" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem
              v-for="model in availableSourceModels"
              :key="model.id"
              :value="model.name"
            >
              {{ model.display_name }} ({{ model.name }})
            </SelectItem>
          </SelectContent>
        </Select>
        <p class="text-xs text-muted-foreground">
          {{ isEditMode ? '创建后不可修改' : '选择要被映射的源模型，计费将按此模型价格' }}
        </p>
      </div>

      <!-- 目标模型选择 -->
      <div class="space-y-2">
        <Label>
          {{ form.mapping_type === 'alias' ? '目标模型 *' : '目标模型 (实际处理请求) *' }}
        </Label>
        <!-- 固定目标模型时显示只读信息 -->
        <div
          v-if="fixedTargetModel"
          class="px-3 py-2 border rounded-md bg-muted/50"
        >
          <span class="font-medium">{{ fixedTargetModel.display_name }}</span>
          <span class="text-muted-foreground ml-1">({{ fixedTargetModel.name }})</span>
        </div>
        <!-- 否则显示下拉选择 -->
        <Select
          v-else
          v-model:open="targetModelSelectOpen"
          :model-value="form.global_model_id"
          @update:model-value="form.global_model_id = $event"
        >
          <SelectTrigger class="w-full">
            <SelectValue placeholder="请选择模型" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem
              v-for="model in availableTargetModels"
              :key="model.id"
              :value="model.id"
            >
              {{ model.display_name }} ({{ model.name }})
            </SelectItem>
          </SelectContent>
        </Select>
      </div>
    </form>

    <template #footer>
      <Button
        type="button"
        variant="outline"
        @click="handleCancel"
      >
        取消
      </Button>
      <Button
        :disabled="submitting"
        @click="handleSubmit"
      >
        <Loader2
          v-if="submitting"
          class="w-4 h-4 mr-2 animate-spin"
        />
        {{ isEditMode ? '保存' : '创建' }}
      </Button>
    </template>
  </Dialog>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import { Loader2, Tag, SquarePen } from 'lucide-vue-next'
import { Dialog, Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '@/components/ui'
import Button from '@/components/ui/button.vue'
import Input from '@/components/ui/input.vue'
import Label from '@/components/ui/label.vue'
import { useToast } from '@/composables/useToast'
import { useFormDialog } from '@/composables/useFormDialog'
import type { ModelAlias, CreateModelAliasRequest, UpdateModelAliasRequest } from '@/api/endpoints/aliases'
import type { GlobalModelResponse } from '@/api/global-models'

export interface ProviderOption {
  id: string
  name: string
  display_name?: string
}

interface AliasFormData {
  alias: string
  global_model_id: string
  provider_id: string | null
  mapping_type: 'alias' | 'mapping'
  is_active: boolean
}

const props = withDefaults(defineProps<{
  open: boolean
  editingAlias?: ModelAlias | null
  globalModels: GlobalModelResponse[]
  providers?: ProviderOption[]
  fixedTargetModel?: GlobalModelResponse | null  // 用于从模型详情抽屉打开时固定目标模型
  fixedProvider?: ProviderOption | null  // 用于 Provider 特定别名固定 Provider
  showProviderSelect?: boolean  // 是否显示 Provider 选择（默认 true）
}>(), {
  editingAlias: null,
  providers: () => [],
  fixedTargetModel: null,
  fixedProvider: null,
  showProviderSelect: true
})

const emit = defineEmits<{
  'update:open': [value: boolean]
  'submit': [data: CreateModelAliasRequest | UpdateModelAliasRequest, isEdit: boolean]
}>()

const { error: showError } = useToast()

// 状态
const submitting = ref(false)
const providerSelectOpen = ref(false)
const sourceModelSelectOpen = ref(false)
const targetModelSelectOpen = ref(false)
const form = ref<AliasFormData>({
  alias: '',
  global_model_id: '',
  provider_id: null,
  mapping_type: 'alias',
  is_active: true,
})

// 处理 Provider 选择变化
function handleProviderChange(value: string) {
  form.value.provider_id = value === 'global' ? null : value
}

// 重置表单
function resetForm() {
  form.value = {
    alias: '',
    global_model_id: props.fixedTargetModel?.id || '',
    provider_id: props.fixedProvider?.id || null,
    mapping_type: 'alias',
    is_active: true,
  }
}

// 加载别名数据（编辑模式）
function loadAliasData() {
  if (!props.editingAlias) return
  form.value = {
    alias: props.editingAlias.alias,
    global_model_id: props.editingAlias.global_model_id,
    provider_id: props.editingAlias.provider_id,
    mapping_type: props.editingAlias.mapping_type || 'alias',
    is_active: props.editingAlias.is_active,
  }
}

// 使用 useFormDialog 统一处理对话框逻辑
const { isEditMode, handleDialogUpdate, handleCancel } = useFormDialog({
  isOpen: () => props.open,
  entity: () => props.editingAlias,
  isLoading: submitting,
  onClose: () => emit('update:open', false),
  loadData: loadAliasData,
  resetForm,
})

// 对话框标题
const dialogTitle = computed(() => {
  if (isEditMode.value) {
    return form.value.mapping_type === 'mapping' ? '编辑映射' : '编辑别名'
  }
  if (props.fixedProvider) {
    return `创建 ${props.fixedProvider.display_name || props.fixedProvider.name} 特定别名/映射`
  }
  return '创建别名/映射'
})

// 对话框描述
const dialogDescription = computed(() => {
  if (isEditMode.value) {
    return form.value.mapping_type === 'mapping' ? '修改模型映射配置' : '修改别名设置'
  }
  return '为模型创建别名或映射规则'
})

// 对话框图标
const dialogIcon = computed(() => isEditMode.value ? SquarePen : Tag)

// 映射模式下可选的源模型（排除已选择的目标模型）
const availableSourceModels = computed(() => {
  return props.globalModels.filter(m => m.id !== form.value.global_model_id)
})

// 可选的目标模型（映射模式下排除已选择的源模型）
const availableTargetModels = computed(() => {
  if (form.value.mapping_type === 'mapping' && form.value.alias) {
    // 找到源模型对应的 GlobalModel
    const sourceModel = props.globalModels.find(m => m.name === form.value.alias)
    if (sourceModel) {
      return props.globalModels.filter(m => m.id !== sourceModel.id)
    }
  }
  return props.globalModels
})

// 提交表单
async function handleSubmit() {
  if (!form.value.alias) {
    showError(form.value.mapping_type === 'alias' ? '请输入别名名称' : '请选择源模型', '错误')
    return
  }

  const targetModelId = props.fixedTargetModel?.id || form.value.global_model_id
  if (!targetModelId) {
    showError('请选择目标模型', '错误')
    return
  }

  submitting.value = true
  try {
    const data: CreateModelAliasRequest | UpdateModelAliasRequest = {
      alias: form.value.alias,
      global_model_id: targetModelId,
      provider_id: props.fixedProvider?.id || form.value.provider_id,
      mapping_type: form.value.mapping_type,
      is_active: form.value.is_active,
    }

    emit('submit', data, !!props.editingAlias)
  } finally {
    submitting.value = false
  }
}
</script>
