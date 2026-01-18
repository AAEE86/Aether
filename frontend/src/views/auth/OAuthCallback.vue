<template>
  <div class="min-h-screen flex items-center justify-center bg-background">
    <div class="text-center space-y-4">
      <div
        v-if="loading"
        class="space-y-4"
      >
        <svg
          class="animate-spin h-8 w-8 mx-auto text-primary"
          xmlns="http://www.w3.org/2000/svg"
          fill="none"
          viewBox="0 0 24 24"
        >
          <circle
            class="opacity-25"
            cx="12"
            cy="12"
            r="10"
            stroke="currentColor"
            stroke-width="4"
          />
          <path
            class="opacity-75"
            fill="currentColor"
            d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
          />
        </svg>
        <p class="text-muted-foreground">正在完成登录...</p>
      </div>
      <div
        v-else-if="error"
        class="space-y-4"
      >
        <div class="text-destructive text-lg font-medium">登录失败</div>
        <p class="text-muted-foreground">{{ error }}</p>
        <button
          class="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
          @click="goHome"
        >
          返回首页
        </button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useAuthStore } from '@/stores/auth'
import { useToast } from '@/composables/useToast'

const router = useRouter()
const authStore = useAuthStore()
const { success: showSuccess, error: showError } = useToast()

const loading = ref(true)
const error = ref('')

function parseHashParams(hash: string): Record<string, string> {
  const params: Record<string, string> = {}
  if (!hash || hash.length <= 1) return params

  const queryString = hash.substring(1) // 移除 #
  const pairs = queryString.split('&')

  for (const pair of pairs) {
    const [key, value] = pair.split('=')
    if (key && value) {
      params[decodeURIComponent(key)] = decodeURIComponent(value)
    }
  }

  return params
}

async function handleOAuthCallback() {
  try {
    // 从 URL fragment 中解析 token
    const hash = window.location.hash
    const params = parseHashParams(hash)

    const accessToken = params.access_token
    const refreshToken = params.refresh_token

    if (!accessToken) {
      error.value = '未收到有效的认证信息'
      loading.value = false
      return
    }

    // 设置 token 到 auth store
    authStore.setTokens(accessToken, refreshToken)

    // 获取用户信息
    await authStore.fetchCurrentUser()

    showSuccess('登录成功，正在跳转...')

    // 清除 URL 中的 token（安全考虑）
    window.history.replaceState({}, document.title, window.location.pathname)

    // 根据用户角色跳转
    const targetPath = authStore.user?.role === 'admin' ? '/admin/dashboard' : '/dashboard'

    setTimeout(() => {
      router.push(targetPath)
    }, 500)

  } catch (e: any) {
    console.error('OAuth callback error:', e)
    error.value = e.message || '登录过程中发生错误'
    loading.value = false
  }
}

function goHome() {
  router.push('/')
}

onMounted(() => {
  handleOAuthCallback()
})
</script>
