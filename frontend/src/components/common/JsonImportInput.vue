<template>
  <div class="space-y-4">
    <input
      ref="fileInputRef"
      type="file"
      :accept="accept"
      multiple
      class="hidden"
      @change="handleFileSelect"
    >

    <div
      v-if="!showManualInput"
      class="rounded-xl border-2 border-dashed transition-colors cursor-pointer"
      :class="isDragging
        ? 'border-primary bg-primary/5'
        : 'border-border hover:border-muted-foreground/40'"
      @click="fileInputRef?.click()"
      @dragover.prevent="isDragging = true"
      @dragleave.prevent="isDragging = false"
      @drop.prevent="handleFileDrop"
    >
      <div class="flex flex-col items-center justify-center py-10 gap-2">
        <div class="w-9 h-9 rounded-full bg-muted/60 flex items-center justify-center">
          <Upload class="w-4 h-4 text-muted-foreground" />
        </div>
        <div class="text-center">
          <p class="text-xs font-medium">
            {{ dropTitle }}
          </p>
          <p class="text-[11px] text-muted-foreground mt-0.5">
            {{ dropHint }}
          </p>
        </div>
      </div>
    </div>

    <div
      v-else
      class="space-y-1.5"
    >
      <Label v-if="manualLabel">
        {{ manualLabel }}
      </Label>
      <Textarea
        :model-value="modelValue"
        :disabled="disabled"
        :placeholder="manualPlaceholder"
        :class="textareaClass"
        spellcheck="false"
        @update:model-value="emit('update:modelValue', $event)"
      />
      <p
        v-if="manualDescription"
        class="text-xs text-muted-foreground"
      >
        {{ manualDescription }}
      </p>
    </div>

    <div class="flex items-center justify-center pt-1">
      <button
        v-if="!showManualInput"
        type="button"
        class="text-sm text-muted-foreground hover:text-foreground transition-colors"
        @click="showManualInput = true"
      >
        {{ pasteToggleText }}
      </button>
      <button
        v-else
        type="button"
        class="text-sm text-muted-foreground hover:text-foreground transition-colors"
        @click="switchToFileMode"
      >
        {{ fileToggleText }}
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Upload } from 'lucide-vue-next'
import { Label, Textarea } from '@/components/ui'

interface ImportInputErrorPayload {
  message: string
  title?: string
}

const props = withDefaults(defineProps<{
  modelValue: string
  disabled?: boolean
  resetKey?: string | number
  accept?: string
  dropTitle?: string
  dropHint?: string
  manualLabel?: string
  manualPlaceholder?: string
  manualDescription?: string
  pasteToggleText?: string
  fileToggleText?: string
  textareaClass?: string
}>(), {
  disabled: false,
  resetKey: '',
  accept: '.json,.txt',
  dropTitle: '拖入导入文件或点击选择',
  dropHint: '支持 .json / .txt，可多选',
  manualLabel: '',
  manualPlaceholder: '',
  manualDescription: '',
  pasteToggleText: '或手动粘贴 JSON',
  fileToggleText: '或选择 JSON 文件导入',
  textareaClass: 'min-h-[220px] text-xs font-mono break-all !rounded-xl',
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
  error: [payload: ImportInputErrorPayload]
}>()

const showManualInput = ref(false)
const isDragging = ref(false)
const fileInputRef = ref<HTMLInputElement | null>(null)

const acceptParts = computed(() => {
  return props.accept
    .split(',')
    .map(part => part.trim().toLowerCase())
    .filter(Boolean)
})

const acceptedExtensions = computed(() => acceptParts.value.filter(part => part.startsWith('.')))
const acceptedMimeTypes = computed(() => acceptParts.value.filter(part => part.includes('/')))

function resetUiState() {
  showManualInput.value = false
  isDragging.value = false
  if (fileInputRef.value) {
    fileInputRef.value.value = ''
  }
}

function emitError(message: string, title?: string) {
  emit('error', { message, title })
}

function isValidFileType(file: File): boolean {
  const name = file.name.toLowerCase()
  const type = (file.type || '').toLowerCase()

  const extensionAllowed = acceptedExtensions.value.length > 0
    && acceptedExtensions.value.some(ext => name.endsWith(ext))

  const mimeAllowed = acceptedMimeTypes.value.length > 0
    && acceptedMimeTypes.value.some((mimeType) => {
      if (mimeType.endsWith('/*')) {
        const prefix = mimeType.slice(0, -1)
        return type.startsWith(prefix)
      }
      return type === mimeType
    })

  if (acceptedExtensions.value.length === 0 && acceptedMimeTypes.value.length === 0) {
    return true
  }
  return extensionAllowed || mimeAllowed
}

function readFileAsText(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = (event) => {
      const content = event.target?.result
      if (typeof content === 'string') {
        resolve(content)
        return
      }
      reject(new Error('读取失败'))
    }
    reader.onerror = () => reject(new Error('读取失败'))
    reader.readAsText(file)
  })
}

function mergeFileContents(contents: string[]): string {
  const items: unknown[] = []

  for (const raw of contents) {
    const trimmed = raw.trim()
    if (!trimmed) continue

    try {
      const parsed = JSON.parse(trimmed)
      if (Array.isArray(parsed)) {
        items.push(...parsed)
      } else {
        items.push(parsed)
      }
      continue
    } catch {
      // Fallback to line mode.
    }

    const lines = trimmed
      .split('\n')
      .map(line => line.trim())
      .filter(line => line && !line.startsWith('#'))
    items.push(...lines)
  }

  if (items.length === 1) {
    return typeof items[0] === 'string' ? items[0] : JSON.stringify(items[0], null, 2)
  }
  return JSON.stringify(items, null, 2)
}

async function readFiles(files: File[]) {
  const validFiles = files.filter(isValidFileType)
  if (validFiles.length === 0) {
    emitError('仅支持 .json 或 .txt 文件', '格式错误')
    return
  }
  if (validFiles.length < files.length) {
    emitError(`已忽略 ${files.length - validFiles.length} 个不支持的文件`, '提示')
  }

  try {
    const contents = await Promise.all(validFiles.map(readFileAsText))
    const merged = validFiles.length === 1 ? contents[0] : mergeFileContents(contents)
    emit('update:modelValue', merged)
    showManualInput.value = true
  } catch {
    emitError('文件读取失败', '错误')
  } finally {
    if (fileInputRef.value) {
      fileInputRef.value.value = ''
    }
  }
}

function handleFileSelect(event: Event) {
  const input = event.target as HTMLInputElement
  const files = input.files
  if (!files || files.length === 0) return
  void readFiles(Array.from(files))
}

function handleFileDrop(event: DragEvent) {
  isDragging.value = false
  const files = event.dataTransfer?.files
  if (!files || files.length === 0) return
  void readFiles(Array.from(files))
}

function switchToFileMode() {
  showManualInput.value = false
  emit('update:modelValue', '')
  if (fileInputRef.value) {
    fileInputRef.value.value = ''
  }
}

watch(
  () => props.resetKey,
  () => {
    resetUiState()
  },
)

watch(
  () => props.modelValue,
  (value) => {
    if (value.trim() && !showManualInput.value) {
      showManualInput.value = true
    }
  },
)
</script>
