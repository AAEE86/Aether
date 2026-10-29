<template>
  <Dialog
    :model-value="open"
    size="lg"
    @update:model-value="(value: boolean) => emit('update:open', value)"
  >
    <template #header>
      <div class="border-b border-border px-6 py-4">
        <div class="flex items-center gap-3">
          <div class="flex h-9 w-9 items-center justify-center rounded-lg bg-primary/10 flex-shrink-0">
            <Terminal class="h-5 w-5 text-primary" />
          </div>
          <div class="flex-1 min-w-0">
            <h3 class="text-lg font-semibold text-foreground leading-tight">
              一键安装并配置 CLI
            </h3>
            <p class="text-xs text-muted-foreground truncate">
              当前密钥：{{ apiKeyLabel }}
            </p>
          </div>
        </div>
      </div>
    </template>

    <div class="space-y-5">
      <div class="rounded-lg border border-border/60 bg-muted/30 p-3 text-xs text-muted-foreground">
        选择要配置的 CLI 和目标系统，Aether 会生成 15 分钟内有效的一次性 install code。页面命令不会包含原始 API Key。
      </div>

      <div class="space-y-2">
        <Label class="text-sm font-semibold">目标 CLI</Label>
        <div class="grid grid-cols-1 sm:grid-cols-3 gap-2">
          <Button
            v-for="option in installCliOptions"
            :key="option.value"
            :variant="installCli === option.value ? 'default' : 'outline'"
            class="justify-start h-auto py-3"
            @click="selectInstallCli(option.value)"
          >
            {{ option.label }}
          </Button>
        </div>
      </div>

      <div class="space-y-2">
        <Label class="text-sm font-semibold">目标系统</Label>
        <div class="grid grid-cols-1 sm:grid-cols-3 gap-2">
          <Button
            v-for="option in installSystemOptions"
            :key="option.value"
            :variant="installSystem === option.value ? 'default' : 'outline'"
            class="justify-start h-auto py-3"
            @click="selectInstallSystem(option.value)"
          >
            {{ option.label }}
          </Button>
        </div>
      </div>

      <div class="space-y-2">
        <div class="flex items-center justify-between gap-2">
          <Label class="text-sm font-semibold">复制到目标机器执行</Label>
          <div class="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              class="gap-1.5"
              :disabled="installLoading || !installCommand"
              :title="installCopied ? '已复制' : '一键复制安装命令'"
              @click="copyInstallCommand"
            >
              <CheckCircle
                v-if="installCopied"
                class="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400"
              />
              <Copy
                v-else
                class="h-3.5 w-3.5"
              />
              {{ installCopied ? '已复制' : '一键复制' }}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              :disabled="installLoading || !apiKey"
              @click="refreshInstallCommand"
            >
              {{ installLoading ? '生成中...' : '重新生成' }}
            </Button>
          </div>
        </div>
        <div class="rounded-lg border border-border/60 bg-background overflow-hidden">
          <pre class="max-h-32 overflow-x-auto whitespace-pre-wrap break-all p-3 text-xs font-mono">{{ installCommand || '正在生成短命令...' }}</pre>
        </div>
        <p class="text-xs text-muted-foreground">
          {{ installCommandHint }}
        </p>
      </div>
    </div>

    <template #footer>
      <Button
        variant="outline"
        class="h-10 px-5"
        @click="emit('update:open', false)"
      >
        关闭
      </Button>
      <Button
        class="h-10 px-5 shadow-lg shadow-primary/20"
        :disabled="!installCommand || installLoading"
        @click="copyInstallCommand"
      >
        {{ installCopied ? '已复制' : '复制命令' }}
      </Button>
    </template>
  </Dialog>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { CheckCircle, Copy, Terminal } from 'lucide-vue-next'
import { Button, Dialog, Label } from '@/components/ui'
import type { ApiKeyInstallSession, InstallSessionTargetSystem, InstallTargetCli } from '@/api/me'
import { useClipboard } from '@/composables/useClipboard'
import { useToast } from '@/composables/useToast'
import { parseApiError } from '@/utils/errorParser'
import { log } from '@/utils/logger'

export interface InstallableApiKey {
  id: string
  name?: string | null
  key_display?: string | null
}

/**
 * 一键安装并配置 CLI：按目标 CLI 与系统生成一次性 install code 命令。
 * 用户端与管理端共用，通过 createSession 传入各自的接口。
 */
const props = defineProps<{
  open: boolean
  apiKey: InstallableApiKey | null
  createSession: (
    apiKeyId: string,
    data: { target_cli: InstallTargetCli; target_system: InstallSessionTargetSystem },
  ) => Promise<ApiKeyInstallSession>
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
}>()

const { success, error: showError } = useToast()
const { copyToClipboard } = useClipboard()

const installCliOptions: Array<{ value: InstallTargetCli; label: string }> = [
  { value: 'claude_code', label: 'Claude Code' },
  { value: 'codex_cli', label: 'Codex CLI' },
  { value: 'gemini_cli', label: 'Gemini CLI' }
]

const installSystemOptions: Array<{ value: InstallSessionTargetSystem; label: string }> = [
  { value: 'macos', label: 'macOS' },
  { value: 'linux', label: 'Linux' },
  { value: 'windows', label: 'Windows' }
]

function detectCurrentSystem(): InstallSessionTargetSystem {
  const platform = window.navigator.platform.toLowerCase()
  const userAgent = window.navigator.userAgent.toLowerCase()
  if (platform.includes('mac')) return 'macos'
  if (platform.includes('win') || userAgent.includes('windows')) return 'windows'
  return 'linux'
}

const installCli = ref<InstallTargetCli>('claude_code')
const installSystem = ref<InstallSessionTargetSystem>(detectCurrentSystem())
const installSession = ref<ApiKeyInstallSession | null>(null)
const installLoading = ref(false)
const installCopied = ref(false)
let installCopiedResetTimer: ReturnType<typeof setTimeout> | null = null
let requestId = 0

const apiKeyLabel = computed(() => props.apiKey?.name || props.apiKey?.key_display || '未选择')

const installCommand = computed(() => {
  if (!installSession.value) return ''
  return installSystem.value === 'windows'
    ? installSession.value.powershell_command
    : installSession.value.unix_command
})

const installCommandHint = computed(() => {
  if (installSystem.value === 'windows') {
    return 'Windows 请在 PowerShell 中执行。install code 使用后立即失效，如需再次执行请重新生成。'
  }
  return 'macOS / Linux 请在 sh 兼容终端中执行。install code 使用后立即失效，如需再次执行请重新生成。'
})

function clearInstallCopiedResetTimer() {
  if (installCopiedResetTimer) {
    clearTimeout(installCopiedResetTimer)
    installCopiedResetTimer = null
  }
}

function resetInstallCopiedState() {
  clearInstallCopiedResetTimer()
  installCopied.value = false
}

async function selectInstallCli(value: InstallTargetCli) {
  installCli.value = value
  await refreshInstallCommand()
}

async function selectInstallSystem(value: InstallSessionTargetSystem) {
  installSystem.value = value
  await refreshInstallCommand()
}

async function refreshInstallCommand() {
  if (!props.apiKey) return
  const currentRequest = ++requestId
  installLoading.value = true
  installSession.value = null
  resetInstallCopiedState()
  try {
    const session = await props.createSession(props.apiKey.id, {
      target_cli: installCli.value,
      target_system: installSystem.value,
    })
    if (currentRequest !== requestId) return
    installSession.value = session
  } catch (err: unknown) {
    if (currentRequest !== requestId) return
    log.error('生成 CLI 安装命令失败:', err)
    showError(parseApiError(err, '生成 CLI 安装命令失败'))
  } finally {
    if (currentRequest === requestId) installLoading.value = false
  }
}

async function copyInstallCommand() {
  if (!installCommand.value) return
  const copied = await copyToClipboard(installCommand.value, false)
  if (!copied) return

  installCopied.value = true
  success('安装命令已复制到剪贴板')
  clearInstallCopiedResetTimer()
  installCopiedResetTimer = setTimeout(() => {
    installCopied.value = false
    installCopiedResetTimer = null
  }, 2000)
}

watch(() => [props.open, props.apiKey?.id] as const, ([open]) => {
  if (!open) {
    requestId += 1
    installSession.value = null
    resetInstallCopiedState()
    return
  }
  void refreshInstallCommand()
}, { immediate: true })

onBeforeUnmount(() => {
  requestId += 1
  resetInstallCopiedState()
})
</script>
