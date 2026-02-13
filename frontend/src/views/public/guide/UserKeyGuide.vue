<script setup lang="ts">
import { RouterLink } from 'vue-router'
import { ArrowRight, Users, Key, Shield, Check, Info, AlertTriangle, Clock } from 'lucide-vue-next'
import { panelClasses } from './guide-config'
import { useSiteInfo } from '@/composables/useSiteInfo'

withDefaults(
  defineProps<{
    baseUrl?: string
  }>(),
  {
    baseUrl: typeof window !== 'undefined' ? window.location.origin : 'https://your-aether.com'
  }
)

const { siteName } = useSiteInfo()

// API Key 字段说明
const keyFields = [
  { name: '名称', description: 'Key 的描述性名称，方便识别', required: true },
  { name: '所属用户', description: 'Key 归属的用户', required: true },
  { name: '有效期', description: '过期时间，不设置则永久有效', required: false },
  { name: '允许的模型', description: '限制该 Key 可访问的模型列表，不设置则可访问所有模型', required: false },
  { name: '请求配额', description: '每日/每月的请求次数限制', required: false },
  { name: 'Token 配额', description: '每日/每月的 Token 用量限制', required: false },
  { name: 'IP 白名单', description: '限制只有特定 IP 可以使用该 Key', required: false },
  { name: '状态', description: '启用/禁用该 Key', required: false }
]

// 权限对比
const roleComparison = [
  { feature: '查看仪表盘', user: true, admin: true },
  { feature: '使用 API', user: true, admin: true },
  { feature: '管理自己的 API Key', user: true, admin: true },
  { feature: '查看用量统计', user: true, admin: true },
  { feature: '管理供应商/端点', user: false, admin: true },
  { feature: '管理模型', user: false, admin: true },
  { feature: '管理其他用户', user: false, admin: true },
  { feature: '管理其他用户的 Key', user: false, admin: true },
  { feature: '系统设置', user: false, admin: true }
]
</script>

<template>
  <div class="space-y-8">
    <!-- 标题 -->
    <div class="space-y-4">
      <h1 class="text-3xl font-bold text-[#262624] dark:text-[#f1ead8]">
        用户与密钥
      </h1>
      <p class="text-lg text-[#666663] dark:text-[#a3a094]">
        用户和 API Key 是 {{ siteName }} 的访问控制核心。通过用户管理分配角色权限，通过 Key 管理控制 API 访问。
      </p>
    </div>

    <!-- 用户管理 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        用户管理
      </h2>

      <div class="grid gap-4 md:grid-cols-2">
        <div
          class="p-5"
          :class="[panelClasses.section]"
        >
          <div class="flex items-center gap-3 mb-3">
            <div class="p-2 rounded-lg bg-blue-500/10">
              <Users class="h-5 w-5 text-blue-500" />
            </div>
            <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
              普通用户
            </h3>
          </div>
          <ul class="space-y-2 text-sm text-[#666663] dark:text-[#a3a094]">
            <li class="flex items-start gap-2">
              <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
              <span>可以使用 API 调用模型</span>
            </li>
            <li class="flex items-start gap-2">
              <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
              <span>管理自己的 API Key</span>
            </li>
            <li class="flex items-start gap-2">
              <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
              <span>查看自己的用量统计</span>
            </li>
          </ul>
        </div>

        <div
          class="p-5"
          :class="[panelClasses.section]"
        >
          <div class="flex items-center gap-3 mb-3">
            <div class="p-2 rounded-lg bg-orange-500/10">
              <Shield class="h-5 w-5 text-orange-500" />
            </div>
            <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
              管理员
            </h3>
          </div>
          <ul class="space-y-2 text-sm text-[#666663] dark:text-[#a3a094]">
            <li class="flex items-start gap-2">
              <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
              <span>管理供应商、端点、模型</span>
            </li>
            <li class="flex items-start gap-2">
              <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
              <span>管理所有用户和 Key</span>
            </li>
            <li class="flex items-start gap-2">
              <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
              <span>系统设置和监控</span>
            </li>
          </ul>
        </div>
      </div>

      <!-- 权限对比表 -->
      <div
        class="overflow-hidden"
        :class="[panelClasses.section]"
      >
        <div class="overflow-x-auto">
          <table class="w-full text-sm">
            <thead>
              <tr class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.12)] bg-[#fafaf7]/50 dark:bg-[#1f1d1a]/50">
                <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                  功能
                </th>
                <th class="px-4 py-3 text-center font-medium text-[#666663] dark:text-[#a3a094]">
                  普通用户
                </th>
                <th class="px-4 py-3 text-center font-medium text-[#666663] dark:text-[#a3a094]">
                  管理员
                </th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="item in roleComparison"
                :key="item.feature"
                class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)] last:border-0"
              >
                <td class="px-4 py-3 text-[#262624] dark:text-[#f1ead8]">
                  {{ item.feature }}
                </td>
                <td class="px-4 py-3 text-center">
                  <Check
                    v-if="item.user"
                    class="h-5 w-5 text-green-500 mx-auto"
                  />
                  <span
                    v-else
                    class="text-[#999]"
                  >—</span>
                </td>
                <td class="px-4 py-3 text-center">
                  <Check
                    v-if="item.admin"
                    class="h-5 w-5 text-green-500 mx-auto"
                  />
                  <span
                    v-else
                    class="text-[#999]"
                  >—</span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </section>

    <!-- API Key 管理 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        API Key 管理
      </h2>

      <div
        class="p-5"
        :class="[panelClasses.section]"
      >
        <div class="flex items-center gap-3 mb-4">
          <div class="p-2 rounded-lg bg-orange-500/10">
            <Key class="h-5 w-5 text-orange-500" />
          </div>
          <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
            什么是 API Key？
          </h3>
        </div>

        <ul class="space-y-3 text-sm text-[#666663] dark:text-[#a3a094]">
          <li class="flex items-start gap-2">
            <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
            <span>API Key 是用户调用 {{ siteName }} API 的凭证</span>
          </li>
          <li class="flex items-start gap-2">
            <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
            <span>一个用户可以有多个 Key，用于不同场景（如开发、生产、测试）</span>
          </li>
          <li class="flex items-start gap-2">
            <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
            <span>可以为 Key 设置细粒度的权限和配额限制</span>
          </li>
          <li class="flex items-start gap-2">
            <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
            <span>Key 的用量会记录到所属用户</span>
          </li>
        </ul>
      </div>
    </section>

    <!-- Key 配置字段 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        Key 配置选项
      </h2>

      <div
        class="overflow-hidden"
        :class="[panelClasses.section]"
      >
        <div class="overflow-x-auto">
          <table class="w-full text-sm">
            <thead>
              <tr class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.12)] bg-[#fafaf7]/50 dark:bg-[#1f1d1a]/50">
                <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                  选项
                </th>
                <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                  说明
                </th>
                <th class="px-4 py-3 text-center font-medium text-[#666663] dark:text-[#a3a094]">
                  必填
                </th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="field in keyFields"
                :key="field.name"
                class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)] last:border-0"
              >
                <td class="px-4 py-3 font-medium text-[#262624] dark:text-[#f1ead8]">
                  {{ field.name }}
                </td>
                <td class="px-4 py-3 text-[#666663] dark:text-[#a3a094]">
                  {{ field.description }}
                </td>
                <td class="px-4 py-3 text-center">
                  <span
                    v-if="field.required"
                    :class="panelClasses.badgeGreen"
                  >必填</span>
                  <span
                    v-else
                    class="text-[#999]"
                  >可选</span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </section>

    <!-- 配额设置 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        配额设置
      </h2>

      <div
        class="p-5 space-y-4"
        :class="[panelClasses.section]"
      >
        <div class="flex items-center gap-3">
          <Clock class="h-5 w-5 text-[#cc785c]" />
          <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
            配额类型
          </h3>
        </div>

        <div class="grid gap-4 md:grid-cols-2">
          <div class="p-4 rounded-lg bg-[#f5f5f0]/50 dark:bg-[#1f1d1a]/50">
            <h4 class="font-medium text-[#262624] dark:text-[#f1ead8]">
              请求次数配额
            </h4>
            <p class="text-sm text-[#666663] dark:text-[#a3a094] mt-1">
              限制每日/每月的 API 调用次数，超过后请求会被拒绝
            </p>
          </div>

          <div class="p-4 rounded-lg bg-[#f5f5f0]/50 dark:bg-[#1f1d1a]/50">
            <h4 class="font-medium text-[#262624] dark:text-[#f1ead8]">
              Token 用量配额
            </h4>
            <p class="text-sm text-[#666663] dark:text-[#a3a094] mt-1">
              限制每日/每月的 Token 消耗量，适合控制成本
            </p>
          </div>
        </div>

        <div class="flex items-start gap-3">
          <Info class="h-5 w-5 text-blue-500 flex-shrink-0 mt-0.5" />
          <div class="text-sm text-[#666663] dark:text-[#a3a094]">
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              配额继承
            </p>
            <p class="mt-1">
              可以在用户级别设置默认配额，新创建的 Key 会自动继承该配额。
              也可以在创建 Key 时覆盖默认配额。
            </p>
          </div>
        </div>
      </div>
    </section>

    <!-- 模型访问控制 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        模型访问控制
      </h2>

      <div
        class="p-5"
        :class="[panelClasses.section]"
      >
        <p class="text-sm text-[#666663] dark:text-[#a3a094] mb-4">
          通过「允许的模型」字段，可以限制 Key 只能访问特定模型：
        </p>

        <ul class="space-y-2 text-sm text-[#666663] dark:text-[#a3a094]">
          <li class="flex items-start gap-2">
            <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
            <span><strong>不设置</strong>：Key 可以访问所有启用的模型</span>
          </li>
          <li class="flex items-start gap-2">
            <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
            <span><strong>设置模型列表</strong>：Key 只能访问列表中的模型</span>
          </li>
        </ul>

        <div class="mt-4 p-4 rounded-lg bg-yellow-500/10 border border-yellow-500/20">
          <div class="flex items-start gap-2">
            <AlertTriangle class="h-5 w-5 text-yellow-600 dark:text-yellow-400 flex-shrink-0 mt-0.5" />
            <div class="text-sm">
              <p class="font-medium text-yellow-700 dark:text-yellow-300">
                使用场景
              </p>
              <p class="text-yellow-600 dark:text-yellow-400 mt-1">
                比如限制免费用户只能使用 gpt-3.5，付费用户可以使用 gpt-4。
                或者为不同项目创建只能访问特定模型的 Key。
              </p>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- 安全建议 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        安全建议
      </h2>

      <div
        class="p-4 space-y-3"
        :class="[panelClasses.section]"
      >
        <div class="flex items-start gap-3">
          <Shield class="h-5 w-5 text-[#cc785c] flex-shrink-0 mt-0.5" />
          <div class="text-sm">
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              设置 Key 有效期
            </p>
            <p class="text-[#666663] dark:text-[#a3a094] mt-1">
              为临时使用的 Key 设置过期时间，避免遗忘造成安全风险。
            </p>
          </div>
        </div>

        <div class="flex items-start gap-3">
          <Shield class="h-5 w-5 text-[#cc785c] flex-shrink-0 mt-0.5" />
          <div class="text-sm">
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              按场景分配 Key
            </p>
            <p class="text-[#666663] dark:text-[#a3a094] mt-1">
              为开发、测试、生产环境分别创建 Key，便于追踪和管理。
            </p>
          </div>
        </div>

        <div class="flex items-start gap-3">
          <Shield class="h-5 w-5 text-[#cc785c] flex-shrink-0 mt-0.5" />
          <div class="text-sm">
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              启用 IP 白名单
            </p>
            <p class="text-[#666663] dark:text-[#a3a094] mt-1">
              对于生产环境的 Key，配置 IP 白名单可以防止 Key 泄露后被滥用。
            </p>
          </div>
        </div>

        <div class="flex items-start gap-3">
          <Shield class="h-5 w-5 text-[#cc785c] flex-shrink-0 mt-0.5" />
          <div class="text-sm">
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              定期审计
            </p>
            <p class="text-[#666663] dark:text-[#a3a094] mt-1">
              定期检查用量统计和审计日志，发现异常及时禁用相关 Key。
            </p>
          </div>
        </div>
      </div>
    </section>

    <!-- 下一步 -->
    <section class="pt-4">
      <RouterLink
        to="/guide/advanced"
        class="p-4 flex items-center gap-3 group"
        :class="[panelClasses.section, panelClasses.cardHover]"
      >
        <div class="flex-1">
          <div class="font-medium text-[#262624] dark:text-[#f1ead8]">
            下一步：高级功能
          </div>
          <div class="text-sm text-[#666663] dark:text-[#a3a094]">
            格式转换、请求头规则等高级配置
          </div>
        </div>
        <ArrowRight class="h-5 w-5 text-[#999] group-hover:text-[#cc785c] transition-colors" />
      </RouterLink>
    </section>
  </div>
</template>
