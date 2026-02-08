<template>
  <Dialog
    :model-value="isOpen"
    title="添加账号"
    :icon="UserPlus"
    size="md"
    @update:model-value="handleDialogUpdate"
  >
    <div class="space-y-4">
      <!-- Tab 切换 -->
      <div class="flex rounded-lg border border-border p-0.5 bg-muted/30">
        <button
          class="flex-1 px-3 py-1.5 text-xs font-medium rounded-md transition-all"
          :disabled="isKiroProvider"
          :class="[
            mode === 'oauth'
              ? 'bg-background text-foreground shadow-sm'
              : 'text-muted-foreground hover:text-foreground',
            isKiroProvider ? 'opacity-50 cursor-not-allowed' : ''
          ]"
          @click="switchMode('oauth')"
        >
          获取授权
        </button>
        <button
          class="flex-1 px-3 py-1.5 text-xs font-medium rounded-md transition-all"
          :class="mode === 'import'
            ? 'bg-background text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground'"
          @click="switchMode('import')"
        >
          导入授权
        </button>
      </div>

      <!-- Tab 内容：grid 叠放，高度取较高者 -->
      <div class="grid [&>*]:col-start-1 [&>*]:row-start-1">
        <!-- ===== 获取授权 ===== -->
        <div
          class="space-y-4 transition-opacity duration-150"
          :class="mode === 'oauth' ? 'opacity-100' : 'opacity-0 pointer-events-none'"
        >
          <div
            v-if="oauth.starting && !oauth.authorization_url"
            class="flex items-center justify-center py-12"
          >
            <div class="text-center">
              <div class="animate-spin rounded-full h-6 w-6 border-b-2 border-primary mx-auto mb-3" />
              <p class="text-xs text-muted-foreground">
                正在准备授权...
              </p>
            </div>
          </div>

          <template v-else-if="oauth.authorization_url">
            <div class="space-y-2">
              <div class="flex items-center gap-2">
                <span class="flex items-center justify-center w-4 h-4 rounded-full bg-primary/10 text-primary text-[10px] font-semibold shrink-0">1</span>
                <span class="text-xs font-medium">前往授权</span>
              </div>
              <div class="flex gap-2 pl-6">
                <Button
                  size="sm"
                  :disabled="oauthBusy"
                  @click="openAuthorizationUrl"
                >
                  <ExternalLink class="w-3 h-3 mr-1" />
                  打开
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  :disabled="oauthBusy"
                  @click="copyToClipboard(oauth.authorization_url)"
                >
                  <Copy class="w-3 h-3 mr-1" />
                  复制
                </Button>
              </div>
            </div>

            <div class="space-y-2">
              <div class="flex items-center gap-2">
                <span class="flex items-center justify-center w-4 h-4 rounded-full bg-primary/10 text-primary text-[10px] font-semibold shrink-0">2</span>
                <span class="text-xs font-medium">粘贴回调 URL</span>
              </div>
              <div class="pl-6">
                <Textarea
                  v-model="oauth.callback_url"
                  :disabled="oauthBusy"
                  placeholder="http://localhost:xxx/callback?code=..."
                  class="min-h-[120px] text-xs font-mono break-all !rounded-xl"
                  spellcheck="false"
                />
              </div>
            </div>
          </template>
        </div>

        <!-- ===== 导入授权 ===== -->
        <div
          class="flex flex-col gap-3 transition-opacity duration-150"
          :class="mode === 'import' ? 'opacity-100' : 'opacity-0 pointer-events-none'"
        >
          <input
            ref="fileInputRef"
            type="file"
            accept=".json"
            class="hidden"
            @change="handleFileSelect"
          >

          <!-- 主区域：拖拽 或 粘贴输入框（同一位置切换） -->
          <div v-if="!importText" class="mt-3">
            <!-- 拖拽模式 -->
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
                <div class="w-8 h-8 rounded-full bg-muted/60 flex items-center justify-center">
                  <Upload class="w-4 h-4 text-muted-foreground" />
                </div>
                <div class="text-center">
                  <p class="text-xs font-medium">
                    拖入授权文件或点击选择
                  </p>
                  <p class="text-[10px] text-muted-foreground mt-0.5">
                    支持 .json 格式
                  </p>
                </div>
              </div>
            </div>

            <!-- 粘贴模式 -->
            <Textarea
              v-else
              v-model="manualPasteText"
              :disabled="importing"
              placeholder="粘贴 Refresh Token 或 JSON 内容"
              class="min-h-[168px] text-xs font-mono break-all !rounded-xl"
              spellcheck="false"
            />
          </div>

          <!-- 底部切换链接：占满剩余空间居中 -->
          <div
            v-if="!importText"
            class="flex-1 flex items-center justify-center"
          >
            <button
              v-if="!showManualInput"
              class="text-xs text-muted-foreground hover:text-foreground transition-colors"
              @click="showManualInput = true"
            >
              或手动粘贴 Refresh Token
            </button>
            <button
              v-else
              class="text-xs text-muted-foreground hover:text-foreground transition-colors"
              @click="showManualInput = false"
            >
              或选择 JSON 文件导入
            </button>
          </div>

          <!-- 已有内容（文件导入后）：显示文本框 -->
          <div v-if="importText" class="space-y-2">
            <div class="flex items-center justify-between">
              <span class="text-xs text-muted-foreground">{{ importFileName || '已粘贴内容' }}</span>
              <button
                class="text-[10px] text-muted-foreground hover:text-foreground transition-colors"
                :disabled="importing"
                @click="clearImport"
              >
                清除
              </button>
            </div>
            <Textarea
              v-model="importText"
              :disabled="importing"
              class="min-h-[160px] text-xs font-mono break-all !rounded-xl"
              spellcheck="false"
            />
          </div>
        </div>
      </div>
    </div>

    <template #footer>
      <Button
        variant="outline"
        @click="handleClose"
      >
        取消
      </Button>
      <Button
        v-if="mode === 'oauth'"
        :disabled="!canCompleteOAuth"
        @click="handleCompleteOAuth"
      >
        {{ oauth.completing ? '验证中...' : '验证' }}
      </Button>
      <Button
        v-else
        :disabled="!canImport"
        @click="handleImport"
      >
        {{ importing ? '导入中...' : '导入' }}
      </Button>
    </template>
  </Dialog>
</template>

<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { Dialog, Button, Textarea } from '@/components/ui'
import { UserPlus, Copy, ExternalLink, Upload } from 'lucide-vue-next'
import { useToast } from '@/composables/useToast'
import { useClipboard } from '@/composables/useClipboard'
import { parseApiError } from '@/utils/errorParser'
import {
  startProviderLevelOAuth,
  completeProviderLevelOAuth,
  importProviderRefreshToken,
  batchImportOAuth,
} from '@/api/endpoints'

const props = defineProps<{
  open: boolean
  providerId: string | null
  providerType: string | null
}>()

const emit = defineEmits<{
  close: []
  saved: []
}>()

const { success, error: showError } = useToast()
const { copyToClipboard } = useClipboard()

// 模式
type DialogMode = 'oauth' | 'import'
const mode = ref<DialogMode>('oauth')

// OAuth 状态
interface OAuthState {
  authorization_url: string
  redirect_uri: string
  instructions: string
  provider_type: string
  callback_url: string
  starting: boolean
  completing: boolean
}

function createInitialOAuthState(): OAuthState {
  return {
    authorization_url: '',
    redirect_uri: '',
    instructions: '',
    provider_type: '',
    callback_url: '',
    starting: false,
    completing: false,
  }
}

const oauth = ref<OAuthState>(createInitialOAuthState())

// 导入状态
const importText = ref('')
const importFileName = ref('')
const manualPasteText = ref('')
const importing = ref(false)
const isDragging = ref(false)
const showManualInput = ref(false)
const fileInputRef = ref<HTMLInputElement | null>(null)

const isOpen = computed(() => props.open)

const isKiroProvider = computed(() => (props.providerType || '').toLowerCase() === 'kiro')


const oauthBusy = computed(() =>
  oauth.value.starting || oauth.value.completing
)

const canCompleteOAuth = computed(() => {
  if (!oauth.value.authorization_url) return false
  if (!oauth.value.callback_url.trim()) return false
  return !oauthBusy.value
})

const canImport = computed(() => {
  const text = importText.value || manualPasteText.value
  return text.trim().length > 0 && !importing.value
})

function resetForm() {
  oauth.value = createInitialOAuthState()
  importText.value = ''
  importFileName.value = ''
  manualPasteText.value = ''
  importing.value = false
  isDragging.value = false
  showManualInput.value = false
  mode.value = isKiroProvider.value ? 'import' : 'oauth'
  if (fileInputRef.value) {
    fileInputRef.value.value = ''
  }
}

function clearImport() {
  importText.value = ''
  importFileName.value = ''
  manualPasteText.value = ''
  showManualInput.value = false
  if (fileInputRef.value) {
    fileInputRef.value.value = ''
  }
}

function switchMode(newMode: DialogMode) {
  if (mode.value === newMode) return

  if (newMode === 'oauth' && isKiroProvider.value) {
    showError('Kiro \u4e0d\u652f\u6301 OAuth \u6388\u6743\uff0c\u8bf7\u4f7f\u7528\u5bfc\u5165\u6388\u6743', '\u63d0\u793a')
    mode.value = 'import'
    return
  }

  mode.value = newMode
  if (newMode === 'oauth' && !oauth.value.authorization_url && !oauth.value.starting) {
    initOAuth()
  }
}

function handleDialogUpdate(value: boolean) {
  if (!value) {
    handleClose()
  }
}

function handleClose() {
  resetForm()
  emit('close')
}

function openAuthorizationUrl() {
  const url = oauth.value.authorization_url
  if (!url) return
  window.open(url, '_blank', 'noopener,noreferrer')
}

async function initOAuth() {
  if (!props.providerId) return
  if (isKiroProvider.value) return


  oauth.value.starting = true
  try {
    const resp = await startProviderLevelOAuth(props.providerId)
    oauth.value.authorization_url = resp.authorization_url
    oauth.value.redirect_uri = resp.redirect_uri
    oauth.value.instructions = resp.instructions
    oauth.value.provider_type = resp.provider_type
  } catch (err: any) {
    const errorMessage = parseApiError(err, '初始化授权失败')
    showError(errorMessage, '错误')
    mode.value = 'import'
  } finally {
    oauth.value.starting = false
  }
}

async function handleCompleteOAuth() {
  if (!canCompleteOAuth.value || !props.providerId) return
  oauth.value.completing = true
  try {
    await completeProviderLevelOAuth(props.providerId, {
      callback_url: oauth.value.callback_url.trim(),
    })
    success('授权成功，账号已添加')
    emit('saved')
    handleClose()
  } catch (err: any) {
    const errorMessage = parseApiError(err, '完成授权失败')
    showError(errorMessage, '错误')
  } finally {
    oauth.value.completing = false
  }
}

// 检测是否为批量导入格式
function isBatchImport(text: string): boolean {
  const trimmed = text.trim()
  // JSON 数组
  if (trimmed.startsWith('[')) {
    try {
      const parsed = JSON.parse(trimmed)
      return Array.isArray(parsed) && parsed.length > 1
    } catch {
      return false
    }
  }
  // 多行文本（纯 Token 一行一个）
  const lines = trimmed.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'))
  return lines.length > 1
}

function parseImportText(text: string): { refresh_token: string; name?: string } | null {
  const trimmed = text.trim()
  if (!trimmed) return null

  // Kiro: keep full JSON so backend can extract auth_method/region/client_id, etc.
  if (isKiroProvider.value) {
    return { refresh_token: trimmed }
  }

  try {
    const parsed = JSON.parse(trimmed)
    if (typeof parsed === 'object' && parsed !== null) {
      const refreshToken = (parsed as any).refresh_token
      if (typeof refreshToken === 'string' && refreshToken.trim()) {
        return {
          refresh_token: refreshToken.trim(),
          name: (parsed as any).name || (parsed as any).oauth_email || undefined,
        }
      }
      return null
    }
  } catch {
    // Not JSON: treat as raw token.
  }

  return { refresh_token: trimmed }
}

function readFile(file: File) {
  if (!file.name.endsWith('.json') && !file.name.endsWith('.txt') && file.type !== 'application/json' && file.type !== 'text/plain') {
    showError('仅支持 .json 或 .txt 文件', '格式错误')
    return
  }
  importFileName.value = file.name
  const reader = new FileReader()
  reader.onload = (e) => {
    const content = e.target?.result
    if (typeof content === 'string') {
      importText.value = content
    }
  }
  reader.readAsText(file)
}

function handleFileSelect(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (file) readFile(file)
}

function handleFileDrop(event: DragEvent) {
  isDragging.value = false
  const file = event.dataTransfer?.files?.[0]
  if (file) readFile(file)
}

async function handleImport() {
  if (!canImport.value || !props.providerId) return

  const inputText = (importText.value || manualPasteText.value).trim()
  if (!inputText) {
    showError('请输入凭据数据', '格式错误')
    return
  }

  importing.value = true
  try {
    // 检测是否为批量导入
    if (isBatchImport(inputText)) {
      // 批量导入
      const result = await batchImportOAuth(props.providerId, inputText)
      if (result.success > 0) {
        if (result.failed > 0) {
          success(`批量导入完成：成功 ${result.success} 个，失败 ${result.failed} 个`)
        } else {
          success(`批量导入成功：${result.success} 个账号已添加`)
        }
        emit('saved')
        handleClose()
      } else {
        // 全部失败，显示第一个错误
        const firstError = result.results.find(r => r.status === 'error')
        showError(firstError?.error || '批量导入失败', '导入失败')
      }
    } else {
      // 单条导入
      const parsed = parseImportText(inputText)
      if (!parsed) {
        showError('无法解析输入内容，请检查格式', '格式错误')
        return
      }
      await importProviderRefreshToken(props.providerId, parsed)
      success('导入成功，账号已添加')
      emit('saved')
      handleClose()
    }
  } catch (err: any) {
    const errorMessage = parseApiError(err, '导入失败')
    showError(errorMessage, '错误')
  } finally {
    importing.value = false
  }
}

watch(() => props.open, (newOpen) => {
  if (newOpen) {
    if (isKiroProvider.value) {
      mode.value = 'import'
    } else {
      initOAuth()
    }
  } else {
    resetForm()
  }
})
</script>
