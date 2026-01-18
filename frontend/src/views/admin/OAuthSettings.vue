<template>
  <PageContainer>
    <PageHeader
      title="OAuth 登录"
      description="配置 OAuth 认证提供商（如 Linux Do、GitHub、Google 等）"
    />

    <div class="mt-6 space-y-6">
      <!-- 提供商列表 -->
      <CardSection
        title="OAuth 提供商"
        description="管理已配置的 OAuth 认证提供商"
      >
        <template #actions>
          <Button
            size="sm"
            @click="handleAddProvider"
          >
            添加提供商
          </Button>
        </template>

        <div
          v-if="loading"
          class="flex items-center justify-center py-8"
        >
          <svg
            class="animate-spin h-6 w-6 text-primary"
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
        </div>

        <div
          v-else-if="providers.length === 0"
          class="text-center py-8 text-muted-foreground"
        >
          <p>暂无 OAuth 提供商配置</p>
          <p class="text-sm mt-2">点击"添加提供商"开始配置</p>
        </div>

        <div
          v-else
          class="space-y-4"
        >
          <div
            v-for="provider in providers"
            :key="provider.provider_id"
            class="flex items-center justify-between p-4 border rounded-lg"
          >
            <div class="flex items-center gap-4">
              <div
                class="w-10 h-10 rounded-full flex items-center justify-center text-lg font-bold"
                :class="provider.is_enabled ? 'bg-primary/10 text-primary' : 'bg-muted text-muted-foreground'"
              >
                {{ provider.display_name.charAt(0).toUpperCase() }}
              </div>
              <div>
                <div class="font-medium">{{ provider.display_name }}</div>
                <div class="text-sm text-muted-foreground">
                  {{ provider.provider_id }}
                  <span
                    v-if="provider.is_enabled"
                    class="ml-2 text-green-600"
                  >已启用</span>
                  <span
                    v-else
                    class="ml-2 text-muted-foreground"
                  >未启用</span>
                </div>
              </div>
            </div>
            <div class="flex gap-2">
              <Button
                size="sm"
                variant="outline"
                :disabled="testingProvider === provider.provider_id"
                @click="handleTestProvider(provider.provider_id)"
              >
                {{ testingProvider === provider.provider_id ? '测试中...' : '测试' }}
              </Button>
              <Button
                size="sm"
                variant="outline"
                @click="handleEditProvider(provider)"
              >
                编辑
              </Button>
              <Button
                size="sm"
                variant="outline"
                class="text-destructive hover:text-destructive"
                @click="handleDeleteProvider(provider)"
              >
                删除
              </Button>
            </div>
          </div>
        </div>
      </CardSection>

      <!-- 使用说明 -->
      <CardSection
        title="使用说明"
        description="如何配置 OAuth 提供商"
      >
        <div class="prose prose-sm dark:prose-invert max-w-none">
          <ol class="list-decimal list-inside space-y-2 text-sm text-muted-foreground">
            <li>在 OAuth 提供商（如 GitHub、Google、Linux Do 等）创建 OAuth 应用</li>
            <li>获取 Client ID 和 Client Secret</li>
            <li>配置回调地址（Redirect URI），格式为：<code class="px-1 py-0.5 bg-muted rounded">https://your-domain.com/api/auth/oauth/{'{provider_id}'}/callback</code></li>
            <li>在此页面添加提供商配置并填写相关信息</li>
            <li>启用提供商后，登录页面将显示对应的 OAuth 登录按钮</li>
          </ol>
        </div>
      </CardSection>
    </div>

    <!-- 添加/编辑提供商对话框 -->
    <Dialog
      v-model:open="showProviderDialog"
      size="lg"
    >
      <div class="space-y-6">
        <div class="text-center">
          <h2 class="text-xl font-semibold">
            {{ editingProvider ? '编辑 OAuth 提供商' : '添加 OAuth 提供商' }}
          </h2>
        </div>

        <form
          class="space-y-4"
          @submit.prevent="handleSaveProvider"
        >
          <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div class="space-y-2">
              <Label for="provider-id">提供商 ID</Label>
              <Input
                id="provider-id"
                v-model="providerForm.provider_id"
                type="text"
                placeholder="如 github, google, linuxdo"
                :disabled="!!editingProvider"
                required
              />
              <p class="text-xs text-muted-foreground">
                仅允许小写字母、数字和下划线，用于 URL 路径
              </p>
            </div>

            <div class="space-y-2">
              <Label for="display-name">显示名称</Label>
              <Input
                id="display-name"
                v-model="providerForm.display_name"
                type="text"
                placeholder="如 GitHub, Google, Linux Do"
                required
              />
            </div>
          </div>

          <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div class="space-y-2">
              <Label for="client-id">Client ID</Label>
              <Input
                id="client-id"
                v-model="providerForm.client_id"
                type="text"
                placeholder="OAuth 应用的 Client ID"
                required
              />
            </div>

            <div class="space-y-2">
              <Label for="client-secret">Client Secret</Label>
              <Input
                id="client-secret"
                v-model="providerForm.client_secret"
                type="password"
                :placeholder="editingProvider?.has_client_secret ? '已设置（留空保持不变）' : 'OAuth 应用的 Client Secret'"
                :required="!editingProvider"
                autocomplete="new-password"
              />
            </div>
          </div>

          <div class="space-y-2">
            <Label for="authorization-url">授权 URL</Label>
            <Input
              id="authorization-url"
              v-model="providerForm.authorization_url"
              type="url"
              placeholder="如 https://github.com/login/oauth/authorize"
              required
            />
          </div>

          <div class="space-y-2">
            <Label for="token-url">Token URL</Label>
            <Input
              id="token-url"
              v-model="providerForm.token_url"
              type="url"
              placeholder="如 https://github.com/login/oauth/access_token"
              required
            />
          </div>

          <div class="space-y-2">
            <Label for="userinfo-url">用户信息 URL</Label>
            <Input
              id="userinfo-url"
              v-model="providerForm.userinfo_url"
              type="url"
              placeholder="如 https://api.github.com/user"
              required
            />
          </div>

          <div class="space-y-2">
            <Label for="redirect-uri">回调地址 (Redirect URI)</Label>
            <Input
              id="redirect-uri"
              v-model="providerForm.redirect_uri"
              type="url"
              placeholder="https://your-domain.com/api/auth/oauth/{provider_id}/callback"
              required
            />
          </div>

          <div class="space-y-2">
            <Label for="frontend-callback-url">前端回调地址</Label>
            <Input
              id="frontend-callback-url"
              v-model="providerForm.frontend_callback_url"
              type="url"
              placeholder="https://your-domain.com/auth/callback"
            />
            <p class="text-xs text-muted-foreground">
              登录成功后重定向到的前端页面
            </p>
          </div>

          <div class="space-y-2">
            <Label for="scope">Scope</Label>
            <Input
              id="scope"
              v-model="providerForm.scope"
              type="text"
              placeholder="如 user:email (多个用空格分隔)"
            />
          </div>

          <div class="space-y-2">
            <Label>用户信息字段映射</Label>
            <p class="text-xs text-muted-foreground mb-2">
              配置如何从 OAuth 提供商的用户信息中提取标准字段
            </p>
            <div class="grid grid-cols-3 gap-2">
              <div>
                <Label
                  for="mapping-id"
                  class="text-xs"
                >用户 ID 字段</Label>
                <Input
                  id="mapping-id"
                  v-model="providerForm.userinfo_mapping.user_id"
                  type="text"
                  placeholder="id"
                />
              </div>
              <div>
                <Label
                  for="mapping-username"
                  class="text-xs"
                >用户名字段</Label>
                <Input
                  id="mapping-username"
                  v-model="providerForm.userinfo_mapping.username"
                  type="text"
                  placeholder="username 或 login"
                />
              </div>
              <div>
                <Label
                  for="mapping-email"
                  class="text-xs"
                >邮箱字段</Label>
                <Input
                  id="mapping-email"
                  v-model="providerForm.userinfo_mapping.email"
                  type="text"
                  placeholder="email"
                />
              </div>
            </div>
          </div>

          <div class="flex items-center justify-between pt-4 border-t">
            <div class="flex items-center gap-2">
              <Switch v-model="providerForm.is_enabled" />
              <Label>启用此提供商</Label>
            </div>
          </div>
        </form>
      </div>

      <template #footer>
        <Button
          variant="outline"
          @click="showProviderDialog = false"
        >
          取消
        </Button>
        <Button
          :disabled="savingProvider"
          @click="handleSaveProvider"
        >
          {{ savingProvider ? '保存中...' : '保存' }}
        </Button>
      </template>
    </Dialog>

    <!-- 删除确认对话框 -->
    <Dialog
      v-model:open="showDeleteDialog"
      size="sm"
    >
      <div class="space-y-4">
        <div class="text-center">
          <h2 class="text-xl font-semibold text-destructive">确认删除</h2>
        </div>
        <p class="text-center text-muted-foreground">
          确定要删除 OAuth 提供商 "{{ deletingProvider?.display_name }}" 吗？
          <br>
          <span class="text-destructive">此操作不可撤销。</span>
        </p>
      </div>

      <template #footer>
        <Button
          variant="outline"
          @click="showDeleteDialog = false"
        >
          取消
        </Button>
        <Button
          variant="destructive"
          :disabled="deletingLoading"
          @click="confirmDeleteProvider"
        >
          {{ deletingLoading ? '删除中...' : '确认删除' }}
        </Button>
      </template>
    </Dialog>
  </PageContainer>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { PageContainer, PageHeader, CardSection } from '@/components/layout'
import { Button, Input, Label, Switch, Dialog } from '@/components/ui'
import { useToast } from '@/composables/useToast'
import { adminApi, type OAuthProviderConfig } from '@/api/admin'

const { success, error } = useToast()

const loading = ref(false)
const providers = ref<OAuthProviderConfig[]>([])
const testingProvider = ref<string | null>(null)

// 添加/编辑对话框
const showProviderDialog = ref(false)
const editingProvider = ref<OAuthProviderConfig | null>(null)
const savingProvider = ref(false)

const defaultForm = (): {
  provider_id: string
  display_name: string
  authorization_url: string
  token_url: string
  userinfo_url: string
  userinfo_mapping: Record<string, string>
  client_id: string
  client_secret: string
  redirect_uri: string
  frontend_callback_url: string
  scope: string
  is_enabled: boolean
} => ({
  provider_id: '',
  display_name: '',
  authorization_url: '',
  token_url: '',
  userinfo_url: '',
  userinfo_mapping: {
    user_id: 'id',
    username: 'username',
    email: 'email'
  },
  client_id: '',
  client_secret: '',
  redirect_uri: '',
  frontend_callback_url: '',
  scope: 'user',
  is_enabled: false
})

const providerForm = ref(defaultForm())

// 删除对话框
const showDeleteDialog = ref(false)
const deletingProvider = ref<OAuthProviderConfig | null>(null)
const deletingLoading = ref(false)

onMounted(async () => {
  await loadProviders()
})

async function loadProviders() {
  loading.value = true
  try {
    const response = await adminApi.getOAuthProviders()
    providers.value = response.providers || []
  } catch (err) {
    error('加载 OAuth 提供商列表失败')
    console.error('加载 OAuth 提供商列表失败:', err)
  } finally {
    loading.value = false
  }
}

function handleAddProvider() {
  editingProvider.value = null
  providerForm.value = defaultForm()
  showProviderDialog.value = true
}

function handleEditProvider(provider: OAuthProviderConfig) {
  editingProvider.value = provider
  providerForm.value = {
    provider_id: provider.provider_id,
    display_name: provider.display_name,
    authorization_url: provider.authorization_url,
    token_url: provider.token_url,
    userinfo_url: provider.userinfo_url,
    userinfo_mapping: { ...provider.userinfo_mapping },
    client_id: provider.client_id,
    client_secret: '',
    redirect_uri: provider.redirect_uri,
    frontend_callback_url: provider.frontend_callback_url || '',
    scope: provider.scope,
    is_enabled: provider.is_enabled
  }
  showProviderDialog.value = true
}

async function handleSaveProvider() {
  // 验证 provider_id 格式
  if (!/^[a-z][a-z0-9_]*$/.test(providerForm.value.provider_id)) {
    error('提供商 ID 格式错误，仅允许小写字母开头，包含小写字母、数字和下划线')
    return
  }

  savingProvider.value = true
  try {
    if (editingProvider.value) {
      // 更新
      const updateData: Record<string, any> = {
        display_name: providerForm.value.display_name,
        authorization_url: providerForm.value.authorization_url,
        token_url: providerForm.value.token_url,
        userinfo_url: providerForm.value.userinfo_url,
        userinfo_mapping: providerForm.value.userinfo_mapping,
        client_id: providerForm.value.client_id,
        redirect_uri: providerForm.value.redirect_uri,
        frontend_callback_url: providerForm.value.frontend_callback_url || null,
        scope: providerForm.value.scope,
        is_enabled: providerForm.value.is_enabled
      }
      if (providerForm.value.client_secret) {
        updateData.client_secret = providerForm.value.client_secret
      }
      await adminApi.updateOAuthProvider(editingProvider.value.provider_id, updateData)
      success('OAuth 提供商配置更新成功')
    } else {
      // 创建
      await adminApi.createOAuthProvider({
        provider_id: providerForm.value.provider_id,
        display_name: providerForm.value.display_name,
        authorization_url: providerForm.value.authorization_url,
        token_url: providerForm.value.token_url,
        userinfo_url: providerForm.value.userinfo_url,
        userinfo_mapping: providerForm.value.userinfo_mapping,
        client_id: providerForm.value.client_id,
        client_secret: providerForm.value.client_secret,
        redirect_uri: providerForm.value.redirect_uri,
        frontend_callback_url: providerForm.value.frontend_callback_url || undefined,
        scope: providerForm.value.scope || undefined,
        is_enabled: providerForm.value.is_enabled
      })
      success('OAuth 提供商创建成功')
    }

    showProviderDialog.value = false
    await loadProviders()
  } catch (err: any) {
    const msg = err.response?.data?.detail || err.message || '保存失败'
    error(`保存 OAuth 提供商失败: ${msg}`)
    console.error('保存 OAuth 提供商失败:', err)
  } finally {
    savingProvider.value = false
  }
}

async function handleTestProvider(providerId: string) {
  testingProvider.value = providerId
  try {
    const response = await adminApi.testOAuthProvider(providerId)
    if (response.success) {
      success(response.message || 'OAuth 连接测试成功')
    } else {
      error(`OAuth 连接测试失败: ${response.message}`)
    }
  } catch (err: any) {
    const msg = err.response?.data?.detail || err.message || '测试失败'
    error(`OAuth 连接测试失败: ${msg}`)
    console.error('OAuth 连接测试失败:', err)
  } finally {
    testingProvider.value = null
  }
}

function handleDeleteProvider(provider: OAuthProviderConfig) {
  deletingProvider.value = provider
  showDeleteDialog.value = true
}

async function confirmDeleteProvider() {
  if (!deletingProvider.value) return

  deletingLoading.value = true
  try {
    await adminApi.deleteOAuthProvider(deletingProvider.value.provider_id)
    success('OAuth 提供商已删除')
    showDeleteDialog.value = false
    await loadProviders()
  } catch (err: any) {
    const msg = err.response?.data?.detail || err.message || '删除失败'
    error(`删除 OAuth 提供商失败: ${msg}`)
    console.error('删除 OAuth 提供商失败:', err)
  } finally {
    deletingLoading.value = false
  }
}
</script>
