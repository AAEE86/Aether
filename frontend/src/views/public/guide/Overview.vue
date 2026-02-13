<script setup lang="ts">
import { RouterLink } from 'vue-router'
import { ArrowRight, Server, Layers, Key, Box, ChevronRight } from 'lucide-vue-next'
import { coreConcepts, apiFormats, configSteps, panelClasses } from './guide-config'
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

// 概念图标映射
const conceptIcons = {
  blue: Server,
  green: Box,
  purple: Layers,
  orange: Key
}

// 概念颜色类
const conceptColors = {
  blue: 'border-blue-500/30 bg-blue-500/5',
  green: 'border-green-500/30 bg-green-500/5',
  purple: 'border-purple-500/30 bg-purple-500/5',
  orange: 'border-orange-500/30 bg-orange-500/5'
}

const conceptIconColors = {
  blue: 'text-blue-500',
  green: 'text-green-500',
  purple: 'text-purple-500',
  orange: 'text-orange-500'
}
</script>

<template>
  <div class="space-y-8">
    <!-- 标题区域 -->
    <div class="space-y-4">
      <h1 class="text-3xl font-bold text-[#262624] dark:text-[#f1ead8]">
        欢迎使用 {{ siteName }}
      </h1>
      <p class="text-lg text-[#666663] dark:text-[#a3a094]">
        {{ siteName }} 是一个 AI API 网关，帮助你统一管理多个 AI 服务供应商，实现负载均衡、访问控制和用量统计。
      </p>
    </div>

    <!-- 核心概念 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        核心概念
      </h2>
      <p class="text-[#666663] dark:text-[#a3a094]">
        理解这四个核心概念，是配置 {{ siteName }} 的基础：
      </p>

      <div class="grid gap-4 sm:grid-cols-2">
        <div
          v-for="concept in coreConcepts"
          :key="concept.name"
          class="p-4 rounded-xl border-2 transition-all"
          :class="[
            conceptColors[concept.color as keyof typeof conceptColors]
          ]"
        >
          <div class="flex items-start gap-3">
            <component
              :is="conceptIcons[concept.color as keyof typeof conceptIcons]"
              class="h-6 w-6 flex-shrink-0 mt-0.5"
              :class="[
                conceptIconColors[concept.color as keyof typeof conceptIconColors]
              ]"
            />
            <div>
              <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
                {{ concept.name }}
              </h3>
              <p class="mt-1 text-sm text-[#666663] dark:text-[#a3a094]">
                {{ concept.description }}
              </p>
            </div>
          </div>
        </div>
      </div>

      <!-- 关系图 -->
      <div
        class="p-6 mt-6"
        :class="[panelClasses.section]"
      >
        <h3 class="text-sm font-medium text-[#666663] dark:text-[#a3a094] mb-4">
          它们之间的关系
        </h3>
        <div class="flex items-center justify-center gap-3 flex-wrap">
          <div class="flex items-center gap-2 px-4 py-2 rounded-lg bg-blue-500/10 border border-blue-500/20">
            <Server class="h-4 w-4 text-blue-500" />
            <span class="text-sm font-medium text-blue-600 dark:text-blue-400">供应商</span>
          </div>
          <ChevronRight class="h-5 w-5 text-[#999] hidden sm:block" />
          <ArrowRight class="h-5 w-5 text-[#999] sm:hidden" />
          <div class="flex items-center gap-2 px-4 py-2 rounded-lg bg-green-500/10 border border-green-500/20">
            <Box class="h-4 w-4 text-green-500" />
            <span class="text-sm font-medium text-green-600 dark:text-green-400">端点</span>
          </div>
          <ChevronRight class="h-5 w-5 text-[#999] hidden sm:block" />
          <ArrowRight class="h-5 w-5 text-[#999] sm:hidden" />
          <div class="flex items-center gap-2 px-4 py-2 rounded-lg bg-purple-500/10 border border-purple-500/20">
            <Layers class="h-4 w-4 text-purple-500" />
            <span class="text-sm font-medium text-purple-600 dark:text-purple-400">模型</span>
          </div>
          <ChevronRight class="h-5 w-5 text-[#999] hidden sm:block" />
          <ArrowRight class="h-5 w-5 text-[#999] sm:hidden" />
          <div class="flex items-center gap-2 px-4 py-2 rounded-lg bg-orange-500/10 border border-orange-500/20">
            <Key class="h-4 w-4 text-orange-500" />
            <span class="text-sm font-medium text-orange-600 dark:text-orange-400">API Key</span>
          </div>
        </div>
        <p class="text-center text-sm text-[#666663] dark:text-[#a3a094] mt-4">
          一个供应商可以有多个端点，一个模型可以关联多个端点，一个 API Key 可以访问多个模型
        </p>
      </div>
    </section>

    <!-- 配置流程 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        配置流程
      </h2>
      <p class="text-[#666663] dark:text-[#a3a094]">
        按照以下步骤完成初始配置：
      </p>

      <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <div
          v-for="step in configSteps"
          :key="step.step"
          class="p-4 relative"
          :class="[panelClasses.section]"
        >
          <div class="absolute -top-3 -left-2 w-8 h-8 rounded-full bg-[#cc785c] flex items-center justify-center text-white font-bold text-sm">
            {{ step.step }}
          </div>
          <div class="pt-2">
            <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
              {{ step.title }}
            </h3>
            <p class="mt-1 text-sm text-[#666663] dark:text-[#a3a094]">
              {{ step.description }}
            </p>
          </div>
        </div>
      </div>
    </section>

    <!-- 支持的 API 格式 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        支持的 API 格式
      </h2>
      <p class="text-[#666663] dark:text-[#a3a094]">
        {{ siteName }} 支持多种 API 格式，可以作为不同客户端的统一入口：
      </p>

      <div
        class="overflow-hidden"
        :class="[panelClasses.section]"
      >
        <div class="overflow-x-auto">
          <table class="w-full text-sm">
            <thead>
              <tr class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.12)] bg-[#fafaf7]/50 dark:bg-[#1f1d1a]/50">
                <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                  格式
                </th>
                <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                  端点
                </th>
                <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                  认证方式
                </th>
                <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                  常用客户端
                </th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="format in apiFormats"
                :key="format.name"
                class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)] last:border-0"
              >
                <td class="px-4 py-3 font-medium text-[#262624] dark:text-[#f1ead8]">
                  {{ format.name }}
                </td>
                <td class="px-4 py-3 font-mono text-xs text-[#666663] dark:text-[#a3a094]">
                  {{ format.endpoint }}
                </td>
                <td class="px-4 py-3 font-mono text-xs text-[#666663] dark:text-[#a3a094]">
                  {{ format.auth }}
                </td>
                <td class="px-4 py-3 text-[#666663] dark:text-[#a3a094]">
                  {{ format.clients.join(', ') }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </section>

    <!-- 快速导航 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        开始配置
      </h2>
      <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        <RouterLink
          to="/guide/provider"
          class="p-4 flex items-center gap-3 group"
          :class="[panelClasses.section, panelClasses.cardHover]"
        >
          <Server class="h-5 w-5 text-[#cc785c]" />
          <div class="flex-1">
            <div class="font-medium text-[#262624] dark:text-[#f1ead8]">
              供应商管理
            </div>
            <div class="text-sm text-[#666663] dark:text-[#a3a094]">
              添加和配置 API 供应商
            </div>
          </div>
          <ArrowRight class="h-5 w-5 text-[#999] group-hover:text-[#cc785c] transition-colors" />
        </RouterLink>

        <RouterLink
          to="/guide/model"
          class="p-4 flex items-center gap-3 group"
          :class="[panelClasses.section, panelClasses.cardHover]"
        >
          <Layers class="h-5 w-5 text-[#cc785c]" />
          <div class="flex-1">
            <div class="font-medium text-[#262624] dark:text-[#f1ead8]">
              模型管理
            </div>
            <div class="text-sm text-[#666663] dark:text-[#a3a094]">
              配置模型和负载均衡
            </div>
          </div>
          <ArrowRight class="h-5 w-5 text-[#999] group-hover:text-[#cc785c] transition-colors" />
        </RouterLink>

        <RouterLink
          to="/guide/user-key"
          class="p-4 flex items-center gap-3 group"
          :class="[panelClasses.section, panelClasses.cardHover]"
        >
          <Key class="h-5 w-5 text-[#cc785c]" />
          <div class="flex-1">
            <div class="font-medium text-[#262624] dark:text-[#f1ead8]">
              用户与密钥
            </div>
            <div class="text-sm text-[#666663] dark:text-[#a3a094]">
              管理用户和 API Key
            </div>
          </div>
          <ArrowRight class="h-5 w-5 text-[#999] group-hover:text-[#cc785c] transition-colors" />
        </RouterLink>
      </div>
    </section>
  </div>
</template>
