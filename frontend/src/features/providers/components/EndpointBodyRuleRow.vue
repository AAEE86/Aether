<template>
  <div
    class="flex items-center gap-1.5 px-2 py-1.5 rounded-md border-l-4 border-muted-foreground/40 bg-muted/30"
    :class="rowClass"
    @dragover.prevent="$emit('dragover')"
    @dragleave="$emit('dragleave')"
    @drop.prevent="$emit('drop')"
  >
    <button
      type="button"
      class="h-7 w-6 shrink-0 inline-flex items-center justify-center rounded-sm text-muted-foreground/60 hover:text-muted-foreground hover:bg-muted cursor-grab active:cursor-grabbing"
      title="拖拽排序"
      draggable="true"
      @dragstart="(e: DragEvent) => $emit('dragstart', e)"
      @dragend="$emit('dragend')"
    >
      <GripVertical class="w-3.5 h-3.5" />
    </button>
    <span
      class="text-[10px] font-semibold text-muted-foreground shrink-0"
      title="请求体"
    >B</span>
    <Switch
      :model-value="rule.enabled"
      class="shrink-0 scale-75 origin-center"
      :title="rule.enabled ? '已启用，点击禁用这条请求体规则' : '已禁用，点击启用这条请求体规则'"
      @update:model-value="(v: boolean) => $emit('update:enabled', v)"
    />
    <Select
      :model-value="rule.action"
      :open="selectOpen"
      @update:model-value="(v: string) => $emit('update:action', v as BodyRuleAction)"
      @update:open="(v) => $emit('update:selectOpen', v)"
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
        <SelectItem value="append">
          追加
        </SelectItem>
        <SelectItem value="insert">
          插入
        </SelectItem>
        <SelectItem value="regex_replace">
          正则替换
        </SelectItem>
      </SelectContent>
    </Select>
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7 shrink-0"
      :class="rule.condition ? 'text-primary' : ''"
      title="条件触发"
      @click="$emit('toggleCondition')"
    >
      <Filter class="w-3 h-3" />
    </Button>
    <template v-if="rule.action === 'set'">
      <Input
        :model-value="rule.path"
        placeholder="字段路径（如 metadata.user_id）"
        size="sm"
        class="flex-1 min-w-0 h-7 text-xs"
        @update:model-value="(v) => $emit('update:field', 'path', v)"
      />
      <span class="text-muted-foreground text-xs">=</span>
      <Input
        :model-value="rule.value"
        placeholder="123 / &quot;text&quot; / {{$original}}"
        size="sm"
        class="flex-1 min-w-0 h-7 text-xs"
        @update:model-value="(v) => $emit('update:field', 'value', v)"
      />
      <CheckCircle
        class="w-4 h-4 shrink-0"
        :class="getBodySetValueValidation(rule) === true ? 'text-green-600' : getBodySetValueValidation(rule) === false ? 'text-destructive' : 'text-muted-foreground/40'"
        :title="getBodySetValueValidationTip(rule)"
      />
    </template>
    <template v-else-if="rule.action === 'drop'">
      <Input
        :model-value="rule.path"
        placeholder="要删除的字段路径"
        size="sm"
        class="flex-1 min-w-0 h-7 text-xs"
        @update:model-value="(v) => $emit('update:field', 'path', v)"
      />
    </template>
    <template v-else-if="rule.action === 'rename'">
      <Input
        :model-value="rule.from"
        placeholder="原路径"
        size="sm"
        class="flex-1 min-w-0 h-7 text-xs"
        @update:model-value="(v) => $emit('update:field', 'from', v)"
      />
      <span class="text-muted-foreground text-xs">→</span>
      <Input
        :model-value="rule.to"
        placeholder="新路径"
        size="sm"
        class="flex-1 min-w-0 h-7 text-xs"
        @update:model-value="(v) => $emit('update:field', 'to', v)"
      />
    </template>
    <template v-else-if="rule.action === 'append'">
      <Input
        :model-value="rule.path"
        placeholder="数组路径（如 messages）"
        size="sm"
        class="flex-[2] min-w-0 h-7 text-xs"
        @update:model-value="(v) => $emit('update:field', 'path', v)"
      />
      <span class="text-muted-foreground text-xs">+=</span>
      <Input
        :model-value="rule.value"
        placeholder="值 (JSON)"
        size="sm"
        class="flex-[3] min-w-0 h-7 text-xs"
        @update:model-value="(v) => $emit('update:field', 'value', v)"
      />
      <CheckCircle
        class="w-4 h-4 shrink-0"
        :class="getBodySetValueValidation(rule) === true ? 'text-green-600' : getBodySetValueValidation(rule) === false ? 'text-destructive' : 'text-muted-foreground/40'"
        :title="getBodySetValueValidationTip(rule)"
      />
    </template>
    <template v-else-if="rule.action === 'insert'">
      <Input
        :model-value="rule.path"
        placeholder="数组路径（如 messages）"
        size="sm"
        class="flex-[2] min-w-0 h-7 text-xs"
        @update:model-value="(v) => $emit('update:field', 'path', v)"
      />
      <Input
        :model-value="rule.index"
        placeholder="位置"
        size="sm"
        class="w-14 h-7 text-xs shrink-0"
        title="插入位置（支持负数）"
        @update:model-value="(v) => $emit('update:field', 'index', v)"
      />
      <Input
        :model-value="rule.value"
        placeholder="值 (JSON)"
        size="sm"
        class="flex-[3] min-w-0 h-7 text-xs"
        @update:model-value="(v) => $emit('update:field', 'value', v)"
      />
      <CheckCircle
        class="w-4 h-4 shrink-0"
        :class="getBodySetValueValidation(rule) === true ? 'text-green-600' : getBodySetValueValidation(rule) === false ? 'text-destructive' : 'text-muted-foreground/40'"
        :title="getBodySetValueValidationTip(rule)"
      />
    </template>
    <template v-else-if="rule.action === 'regex_replace'">
      <Input
        :model-value="rule.path"
        placeholder="字段路径"
        size="sm"
        class="flex-[2] min-w-0 h-7 text-xs"
        @update:model-value="(v) => $emit('update:field', 'path', v)"
      />
      <Input
        :model-value="rule.pattern"
        placeholder="正则"
        size="sm"
        class="flex-[2] min-w-0 h-7 text-xs font-mono"
        @update:model-value="(v) => $emit('update:field', 'pattern', v)"
      />
      <span class="text-muted-foreground text-xs">→</span>
      <Input
        :model-value="rule.replacement"
        placeholder="替换为"
        size="sm"
        class="flex-[2] min-w-0 h-7 text-xs"
        @update:model-value="(v) => $emit('update:field', 'replacement', v)"
      />
      <Input
        :model-value="rule.flags"
        placeholder="ims"
        size="sm"
        class="w-12 h-7 text-xs shrink-0 font-mono"
        title="正则标志：i=忽略大小写 m=多行 s=dotall"
        @update:model-value="(v) => $emit('update:field', 'flags', v)"
      />
      <Input
        :model-value="rule.count"
        placeholder="全部"
        size="sm"
        class="w-14 h-7 text-xs shrink-0"
        title="替换次数；留空=默认全部，0=全部"
        @update:model-value="(v) => $emit('update:field', 'count', v)"
      />
      <CheckCircle
        class="w-4 h-4 shrink-0"
        :class="getRegexPatternValidation(rule) === true ? 'text-green-600' : getRegexPatternValidation(rule) === false ? 'text-destructive' : 'text-muted-foreground/40'"
        :title="getRegexPatternValidationTip(rule)"
      />
    </template>
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7 shrink-0"
      @click="$emit('remove')"
    >
      <X class="w-3 h-3" />
    </Button>
  </div>
  <EndpointConditionEditor
    v-if="rule.condition"
    :model-value="rule.condition"
    :path-hint="getBodyRuleConditionPathPlaceholder(rule.path)"
    removable
    @update:model-value="(condition) => $emit('update:condition', condition)"
    @remove="$emit('clearCondition')"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { CheckCircle, Filter, GripVertical, X } from 'lucide-vue-next'
import {
  Button,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
} from '@/components/ui'
import { useI18n } from '@/i18n'
import EndpointConditionEditor from './EndpointConditionEditor.vue'
import { getBodyRuleConditionPathPlaceholder } from './endpoint-rule-condition'
import type { EditableConditionNode } from './endpoint-rule-condition'
import {
  getBodySetValueValidation,
  getRegexPatternValidation,
  prepareValueForJsonParse,
  restoreOriginalPlaceholder,
} from './endpoint-rule-editing'
import type { BodyRuleAction, BodyRuleEditableField, EditableBodyRule } from './endpoint-rule-editing'

/**
 * 单条请求体规则的编辑行：拖拽把手、启用开关、动作选择、条件触发、
 * 按动作切换的字段输入、JSON / 正则校验提示与删除。
 */
const props = withDefaults(defineProps<{
  rule: EditableBodyRule
  selectOpen?: boolean
  dragging?: boolean
  dragOver?: boolean
}>(), {
  selectOpen: false,
  dragging: false,
  dragOver: false,
})

defineEmits<{
  dragover: []
  dragleave: []
  drop: []
  dragstart: [event: DragEvent]
  dragend: []
  'update:enabled': [value: boolean]
  'update:action': [value: BodyRuleAction]
  'update:selectOpen': [value: boolean]
  'update:field': [field: BodyRuleEditableField, value: string]
  'update:condition': [condition: EditableConditionNode]
  toggleCondition: []
  clearCondition: []
  remove: []
}>()

const { legacyT } = useI18n()

const rowClass = computed(() => [
  !props.rule.enabled ? 'opacity-60 border-muted-foreground/25 bg-muted/20' : '',
  props.dragging ? 'opacity-60 border-muted-foreground/70 bg-muted/50' : '',
  props.dragOver ? 'ring-1 ring-muted-foreground/40 bg-muted/40' : '',
])

// 获取正则验证提示
function getRegexPatternValidationTip(rule: EditableBodyRule): string {
  const validation = getRegexPatternValidation(rule)
  if (validation === null) return legacyT('输入正则表达式')
  if (validation === true) return legacyT('有效的正则表达式')
  try {
    new RegExp(rule.pattern.trim())
    // 正则有效但 flags 无效
    return legacyT('无效的 flags（仅允许 i/m/s）')
  } catch (err: unknown) {
    return err instanceof Error ? err.message : String(err)
  }
}

// 获取验证提示
function getBodySetValueValidationTip(rule: EditableBodyRule): string {
  const validation = getBodySetValueValidation(rule)
  if (validation === null) return legacyT('点击验证 JSON')
  if (validation === true) {
    const parsed = restoreOriginalPlaceholder(JSON.parse(prepareValueForJsonParse(rule.value.trim())))
    const type = Array.isArray(parsed)
      ? legacyT('数组')
      : typeof parsed === 'object' && parsed !== null
        ? legacyT('对象')
        : typeof parsed === 'string'
          ? legacyT('字符串')
          : typeof parsed === 'number'
            ? legacyT('数字')
            : typeof parsed === 'boolean'
              ? legacyT('布尔')
              : 'null'
    return `${legacyT('有效的 JSON')} (${type})`
  }
  try {
    JSON.parse(prepareValueForJsonParse(rule.value.trim()))
    return ''
  } catch (err: unknown) {
    return err instanceof Error ? err.message : String(err)
  }
}
</script>
