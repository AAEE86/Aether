<script setup lang="ts">
import { ref, computed } from 'vue'
import { Search, ChevronDown, ExternalLink, HelpCircle } from 'lucide-vue-next'
import { faqItems, panelClasses } from './guide-config'
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

// 搜索关键词
const searchQuery = ref('')

// 展开的 FAQ
const expandedIds = ref<Set<string>>(new Set())

// 过滤后的 FAQ
const filteredFaqs = computed(() => {
  if (!searchQuery.value.trim()) {
    return faqItems
  }
  const query = searchQuery.value.toLowerCase()
  return faqItems.filter(
    item =>
      item.question.toLowerCase().includes(query) ||
      item.answer.toLowerCase().includes(query)
  )
})

// 按分类分组
const faqsByCategory = computed(() => {
  const grouped: Record<string, typeof faqItems> = {}
  for (const item of filteredFaqs.value) {
    if (!grouped[item.category]) {
      grouped[item.category] = []
    }
    grouped[item.category].push(item)
  }
  return grouped
})

// 切换展开状态
function toggleExpand(id: string) {
  if (expandedIds.value.has(id)) {
    expandedIds.value.delete(id)
  } else {
    expandedIds.value.add(id)
  }
}

// 全部展开/收起
function toggleAll() {
  if (expandedIds.value.size === filteredFaqs.value.length) {
    expandedIds.value.clear()
  } else {
    expandedIds.value = new Set(filteredFaqs.value.map(item => item.id))
  }
}
</script>

<template>
  <div class="space-y-8">
    <!-- 标题 -->
    <div class="space-y-4">
      <h1 class="text-3xl font-bold text-[#262624] dark:text-[#f1ead8]">
        常见问题
      </h1>
      <p class="text-lg text-[#666663] dark:text-[#a3a094]">
        关于 {{ siteName }} 使用和配置的常见问题解答。
      </p>
    </div>

    <!-- 搜索栏 -->
    <div
      class="p-4"
      :class="[panelClasses.section]"
    >
      <div class="flex items-center gap-3">
        <Search class="h-5 w-5 text-[#999]" />
        <input
          v-model="searchQuery"
          type="text"
          placeholder="搜索问题..."
          class="flex-1 bg-transparent border-none outline-none text-[#262624] dark:text-[#f1ead8] placeholder:text-[#999]"
        >
        <button
          v-if="filteredFaqs.length > 0"
          class="text-sm text-[#cc785c] hover:underline"
          @click="toggleAll"
        >
          {{ expandedIds.size === filteredFaqs.length ? '全部收起' : '全部展开' }}
        </button>
      </div>
    </div>

    <!-- FAQ 列表 -->
    <div
      v-if="filteredFaqs.length > 0"
      class="space-y-6"
    >
      <div
        v-for="category in Object.keys(faqsByCategory)"
        :key="category"
        class="space-y-3"
      >
        <h2 class="text-lg font-semibold text-[#262624] dark:text-[#f1ead8] flex items-center gap-2">
          <HelpCircle class="h-5 w-5 text-[#cc785c]" />
          {{ category }}
        </h2>

        <div class="space-y-2">
          <div
            v-for="faq in faqsByCategory[category]"
            :key="faq.id"
            class="overflow-hidden"
            :class="[panelClasses.section]"
          >
            <button
              class="w-full px-4 py-3 flex items-center justify-between text-left hover:bg-[#f5f5f0]/50 dark:hover:bg-[#1f1d1a]/50 transition-colors"
              @click="toggleExpand(faq.id)"
            >
              <span class="font-medium text-[#262624] dark:text-[#f1ead8] pr-4">
                {{ faq.question }}
              </span>
              <ChevronDown
                class="h-5 w-5 text-[#999] flex-shrink-0 transition-transform"
                :class="{ 'rotate-180': expandedIds.has(faq.id) }"
              />
            </button>
            <div
              v-show="expandedIds.has(faq.id)"
              class="px-4 pb-4 text-sm text-[#666663] dark:text-[#a3a094] whitespace-pre-line"
            >
              {{ faq.answer }}
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 无结果 -->
    <div
      v-else
      class="p-8 text-center"
      :class="[panelClasses.section]"
    >
      <HelpCircle class="h-12 w-12 text-[#999] mx-auto mb-4" />
      <p class="text-[#666663] dark:text-[#a3a094]">
        没有找到匹配的问题，请尝试其他关键词
      </p>
    </div>

    <!-- 更多帮助 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        需要更多帮助？
      </h2>

      <div class="grid gap-4 md:grid-cols-2">
        <a
          href="https://github.com/your-repo/aether"
          target="_blank"
          rel="noopener noreferrer"
          class="p-4 flex items-center gap-3 group"
          :class="[panelClasses.section, panelClasses.cardHover]"
        >
          <div class="p-2 rounded-lg bg-gray-500/10">
            <svg
              class="h-5 w-5"
              viewBox="0 0 24 24"
              fill="currentColor"
            >
              <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
            </svg>
          </div>
          <div class="flex-1">
            <div class="font-medium text-[#262624] dark:text-[#f1ead8]">GitHub</div>
            <div class="text-sm text-[#666663] dark:text-[#a3a094]">查看源码、提交 Issue</div>
          </div>
          <ExternalLink class="h-4 w-4 text-[#999] group-hover:text-[#cc785c] transition-colors" />
        </a>

        <a
          href="https://github.com/your-repo/aether/discussions"
          target="_blank"
          rel="noopener noreferrer"
          class="p-4 flex items-center gap-3 group"
          :class="[panelClasses.section, panelClasses.cardHover]"
        >
          <div class="p-2 rounded-lg bg-blue-500/10">
            <svg
              class="h-5 w-5 text-blue-500"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
            >
              <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
            </svg>
          </div>
          <div class="flex-1">
            <div class="font-medium text-[#262624] dark:text-[#f1ead8]">Discussions</div>
            <div class="text-sm text-[#666663] dark:text-[#a3a094]">社区讨论、功能建议</div>
          </div>
          <ExternalLink class="h-4 w-4 text-[#999] group-hover:text-[#cc785c] transition-colors" />
        </a>
      </div>
    </section>
  </div>
</template>
