<template>
  <PageContainer>
    <PageHeader
      title="LDAP 配置"
      description="配置 LDAP 认证服务"
    />

    <div class="mt-6 space-y-6">
      <CardSection
        title="LDAP 服务器配置"
        description="配置 LDAP 服务器连接参数"
      >
        <template #actions>
          <div class="flex gap-2">
            <Button
              size="sm"
              variant="outline"
              :disabled="testLoading"
              @click="handleTestConnection"
            >
              {{ testLoading ? '测试中...' : '测试连接' }}
            </Button>
            <Button
              size="sm"
              :disabled="saveLoading"
              @click="handleSave"
            >
              {{ saveLoading ? '保存中...' : '保存' }}
            </Button>
          </div>
        </template>

        <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div>
            <Label for="server-url" class="block text-sm font-medium">
              服务器地址
            </Label>
            <Input
              id="server-url"
              v-model="ldapConfig.server_url"
              type="text"
              placeholder="ldap://ldap.example.com:389"
              class="mt-1"
            />
            <p class="mt-1 text-xs text-muted-foreground">
              格式: ldap://host:389 或 ldaps://host:636
            </p>
          </div>

          <div>
            <Label for="bind-dn" class="block text-sm font-medium">
              绑定 DN
            </Label>
            <Input
              id="bind-dn"
              v-model="ldapConfig.bind_dn"
              type="text"
              placeholder="cn=admin,dc=example,dc=com"
              class="mt-1"
            />
            <p class="mt-1 text-xs text-muted-foreground">
              用于连接 LDAP 服务器的管理员 DN
            </p>
          </div>

          <div>
            <Label for="bind-password" class="block text-sm font-medium">
              绑定密码
            </Label>
            <div class="relative mt-1">
              <Input
                id="bind-password"
                v-model="ldapConfig.bind_password"
                type="password"
                :placeholder="hasPassword ? '已设置（留空保持不变）' : '请输入密码'"
                :class="(hasPassword || ldapConfig.bind_password) ? 'pr-10' : ''"
                autocomplete="new-password"
              />
              <button
                v-if="hasPassword || ldapConfig.bind_password"
                type="button"
                class="absolute right-3 top-1/2 -translate-y-1/2 p-1 rounded-full text-muted-foreground/60 hover:text-muted-foreground hover:bg-muted/50 transition-colors"
                @click="handleClearPassword"
                title="清除密码"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
              </button>
            </div>
            <p class="mt-1 text-xs text-muted-foreground">
              绑定账号的密码
            </p>
          </div>

          <div>
            <Label for="base-dn" class="block text-sm font-medium">
              基础 DN
            </Label>
            <Input
              id="base-dn"
              v-model="ldapConfig.base_dn"
              type="text"
              placeholder="ou=users,dc=example,dc=com"
              class="mt-1"
            />
            <p class="mt-1 text-xs text-muted-foreground">
              用户搜索的基础 DN
            </p>
          </div>

          <div>
            <Label for="user-search-filter" class="block text-sm font-medium">
              用户搜索过滤器
            </Label>
            <Input
              id="user-search-filter"
              v-model="ldapConfig.user_search_filter"
              type="text"
              placeholder="(uid={username})"
              class="mt-1"
            />
            <p class="mt-1 text-xs text-muted-foreground">
              {username} 会被替换为登录用户名
            </p>
          </div>

          <div>
            <Label for="username-attr" class="block text-sm font-medium">
              用户名属性
            </Label>
            <Input
              id="username-attr"
              v-model="ldapConfig.username_attr"
              type="text"
              placeholder="uid"
              class="mt-1"
            />
            <p class="mt-1 text-xs text-muted-foreground">
              常用: uid (OpenLDAP), sAMAccountName (AD)
            </p>
          </div>

          <div>
            <Label for="email-attr" class="block text-sm font-medium">
              邮箱属性
            </Label>
            <Input
              id="email-attr"
              v-model="ldapConfig.email_attr"
              type="text"
              placeholder="mail"
              class="mt-1"
            />
          </div>

          <div>
            <Label for="display-name-attr" class="block text-sm font-medium">
              显示名称属性
            </Label>
            <Input
              id="display-name-attr"
              v-model="ldapConfig.display_name_attr"
              type="text"
              placeholder="cn"
              class="mt-1"
            />
          </div>

          <div>
            <Label for="connect-timeout" class="block text-sm font-medium">
              连接超时 (秒)
            </Label>
            <Input
              id="connect-timeout"
              v-model.number="ldapConfig.connect_timeout"
              type="number"
              min="1"
              max="60"
              placeholder="10"
              class="mt-1"
            />
            <p class="mt-1 text-xs text-muted-foreground">
              单次 LDAP 操作超时时间 (1-60秒)，跨国网络建议 15-30 秒
            </p>
          </div>
        </div>

        <div class="mt-6 space-y-4">
          <div class="flex items-center justify-between">
            <div>
              <Label class="text-sm font-medium">使用 STARTTLS</Label>
              <p class="text-xs text-muted-foreground">
                在非 SSL 连接上启用 TLS 加密
              </p>
            </div>
            <Switch v-model="ldapConfig.use_starttls" />
          </div>

          <div class="flex items-center justify-between">
            <div>
              <Label class="text-sm font-medium">启用 LDAP 认证</Label>
              <p class="text-xs text-muted-foreground">
                允许用户使用 LDAP 账号登录
              </p>
            </div>
            <Switch v-model="ldapConfig.is_enabled" />
          </div>

          <div class="flex items-center justify-between">
            <div>
              <Label class="text-sm font-medium">仅允许 LDAP 登录</Label>
              <p class="text-xs text-muted-foreground">
                禁用本地账号登录，仅允许 LDAP 认证
              </p>
            </div>
            <Switch v-model="ldapConfig.is_exclusive" />
          </div>
        </div>
      </CardSection>
    </div>
  </PageContainer>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { PageContainer, PageHeader, CardSection } from '@/components/layout'
import { Button, Input, Label, Switch } from '@/components/ui'
import { useToast } from '@/composables/useToast'
import { adminApi, type LdapConfigUpdateRequest } from '@/api/admin'

const { success, error } = useToast()

const loading = ref(false)
const saveLoading = ref(false)
const testLoading = ref(false)
const hasPassword = ref(false)
const clearPassword = ref(false) // 标记是否要清除密码

const ldapConfig = ref({
  server_url: '',
  bind_dn: '',
  bind_password: '',
  base_dn: '',
  user_search_filter: '(uid={username})',
  username_attr: 'uid',
  email_attr: 'mail',
  display_name_attr: 'cn',
  is_enabled: false,
  is_exclusive: false,
  use_starttls: false,
  connect_timeout: 10,
})

onMounted(async () => {
  await loadConfig()
})

async function loadConfig() {
  loading.value = true
  try {
    const response = await adminApi.getLdapConfig()
    ldapConfig.value = {
      server_url: response.server_url || '',
      bind_dn: response.bind_dn || '',
      bind_password: '',
      base_dn: response.base_dn || '',
      user_search_filter: response.user_search_filter || '(uid={username})',
      username_attr: response.username_attr || 'uid',
      email_attr: response.email_attr || 'mail',
      display_name_attr: response.display_name_attr || 'cn',
      is_enabled: response.is_enabled || false,
      is_exclusive: response.is_exclusive || false,
      use_starttls: response.use_starttls || false,
      connect_timeout: response.connect_timeout || 10,
    }
    hasPassword.value = !!response.has_bind_password
    clearPassword.value = false
  } catch (err) {
    error('加载 LDAP 配置失败')
    console.error('加载 LDAP 配置失败:', err)
  } finally {
    loading.value = false
  }
}

async function handleSave() {
  saveLoading.value = true
  try {
    const payload: LdapConfigUpdateRequest = {
      server_url: ldapConfig.value.server_url,
      bind_dn: ldapConfig.value.bind_dn,
      base_dn: ldapConfig.value.base_dn,
      user_search_filter: ldapConfig.value.user_search_filter,
      username_attr: ldapConfig.value.username_attr,
      email_attr: ldapConfig.value.email_attr,
      display_name_attr: ldapConfig.value.display_name_attr,
      is_enabled: ldapConfig.value.is_enabled,
      is_exclusive: ldapConfig.value.is_exclusive,
      use_starttls: ldapConfig.value.use_starttls,
      connect_timeout: ldapConfig.value.connect_timeout,
    }

    // 优先使用输入的新密码；否则如果标记清除则发送空字符串
    let passwordAction: 'unchanged' | 'updated' | 'cleared' = 'unchanged'
    if (ldapConfig.value.bind_password) {
      payload.bind_password = ldapConfig.value.bind_password
      passwordAction = 'updated'
    } else if (clearPassword.value) {
      payload.bind_password = ''
      passwordAction = 'cleared'
    }

    await adminApi.updateLdapConfig(payload)
    success('LDAP 配置保存成功')

    if (passwordAction === 'cleared') {
      hasPassword.value = false
      clearPassword.value = false
    } else if (passwordAction === 'updated') {
      hasPassword.value = true
      clearPassword.value = false
    }
    ldapConfig.value.bind_password = ''
  } catch (err) {
    error('保存 LDAP 配置失败')
    console.error('保存 LDAP 配置失败:', err)
  } finally {
    saveLoading.value = false
  }
}

async function handleTestConnection() {
  if (clearPassword.value && !ldapConfig.value.bind_password) {
    error('已标记清除绑定密码，请先保存或输入新的绑定密码再测试')
    return
  }

  testLoading.value = true
  try {
    const payload: LdapConfigUpdateRequest = {
      server_url: ldapConfig.value.server_url,
      bind_dn: ldapConfig.value.bind_dn,
      base_dn: ldapConfig.value.base_dn,
      user_search_filter: ldapConfig.value.user_search_filter,
      username_attr: ldapConfig.value.username_attr,
      email_attr: ldapConfig.value.email_attr,
      display_name_attr: ldapConfig.value.display_name_attr,
      is_enabled: ldapConfig.value.is_enabled,
      is_exclusive: ldapConfig.value.is_exclusive,
      use_starttls: ldapConfig.value.use_starttls,
      connect_timeout: ldapConfig.value.connect_timeout,
      ...(ldapConfig.value.bind_password && { bind_password: ldapConfig.value.bind_password }),
    }
    const response = await adminApi.testLdapConnection(payload)
    if (response.success) {
      success('LDAP 连接测试成功')
    } else {
      error(`LDAP 连接测试失败: ${response.message}`)
    }
  } catch (err) {
    error('LDAP 连接测试失败')
    console.error('LDAP 连接测试失败:', err)
  } finally {
    testLoading.value = false
  }
}

function handleClearPassword() {
  // 如果有输入内容，先清空输入框
  if (ldapConfig.value.bind_password) {
    ldapConfig.value.bind_password = ''
    return
  }
  // 标记要清除服务端密码（保存时生效）
  if (hasPassword.value) {
    clearPassword.value = true
    hasPassword.value = false
  }
}
</script>
