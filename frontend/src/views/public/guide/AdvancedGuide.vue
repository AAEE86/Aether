<script setup lang="ts">
import { RouterLink } from 'vue-router'
import { ArrowRight, Shuffle, FileCode, Globe, Shield, Check, Info, AlertTriangle, Settings } from 'lucide-vue-next'
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

// 格式转换示例
const conversionExamples = [
  {
    from: 'OpenAI SDK',
    to: 'Claude API',
    description: '用 OpenAI SDK 调用 Claude 模型',
    icon: 'openai'
  },
  {
    from: 'Anthropic SDK',
    to: 'OpenAI API',
    description: '用 Claude SDK 调用 GPT 模型',
    icon: 'claude'
  },
  {
    from: 'Claude Code',
    to: 'OpenAI API',
    description: 'Claude Code 使用 GPT 模型',
    icon: 'cli'
  }
]

// 请求头规则类型
const headerRuleTypes = [
  { type: 'add', name: '添加', description: '添加新的请求头，如果已存在则跳过' },
  { type: 'set', name: '设置', description: '设置请求头的值，会覆盖已有值' },
  { type: 'remove', name: '删除', description: '删除指定的请求头' }
]

// 请求体规则类型
const bodyRuleTypes = [
  { action: 'set', name: '覆写', description: '设置或覆盖指定路径的字段值' },
  { action: 'drop', name: '删除', description: '删除指定路径的字段' },
  { action: 'rename', name: '重命名', description: '将字段从一个路径移动到另一个路径' },
  { action: 'insert', name: '插入', description: '在数组的指定位置插入元素，位置留空则追加到末尾' },
  { action: 'regex_replace', name: '正则替换', description: '对字符串字段执行正则表达式替换' },
]

// 请求体规则示例
const bodyRuleExamples = [
  {
    title: '注入系统提示词',
    description: '在 messages 数组开头插入一条 system 消息（index: 0）',
    rule: `{
  "action": "insert",
  "path": "messages",
  "index": 0,
  "value": {
    "role": "system",
    "content": "你是一个专业助手"
  }
}`,
  },
  {
    title: '追加消息到末尾',
    description: '不指定 index，自动追加到数组末尾',
    rule: `{
  "action": "insert",
  "path": "messages",
  "value": {
    "role": "user",
    "content": "请用中文回答"
  }
}`,
  },
  {
    title: '设置自定义元数据',
    description: '覆写嵌套字段，不存在时自动创建中间层级',
    rule: `{
  "action": "set",
  "path": "metadata.source",
  "value": "internal-app"
}`,
  },
  {
    title: '删除不需要的字段',
    description: '移除请求体中的敏感或多余字段',
    rule: `{
  "action": "drop",
  "path": "user_info.ip_address"
}`,
  },
  {
    title: '内容脱敏',
    description: '用正则替换 messages 中的手机号',
    rule: `{
  "action": "regex_replace",
  "path": "messages[-1].content",
  "pattern": "1[3-9]\\\\d{9}",
  "replacement": "[手机号已隐藏]",
  "flags": ""
}`,
  },
  {
    title: '重命名字段',
    description: '将字段从旧路径移动到新路径',
    rule: `{
  "action": "rename",
  "from": "extra.custom_id",
  "to": "metadata.trace_id"
}`,
  },
]

// 路径语法示例
const pathSyntaxExamples = [
  { path: 'metadata.user', desc: '嵌套 dict 字段' },
  { path: 'messages[0].content', desc: '数组第一个元素的 content 字段' },
  { path: 'messages[-1]', desc: '数组最后一个元素' },
  { path: 'matrix[0][1]', desc: '多维数组访问' },
  { path: 'config\\.v1.enabled', desc: '\\. 转义为字面量点号 → key "config.v1"' },
]

// 系统设置分类
const systemSettings = [
  {
    category: '基础设置',
    items: [
      { name: '系统名称', description: '显示在页面标题和 Logo 旁边的名称' },
      { name: '注册开关', description: '是否允许新用户注册' },
      { name: '默认用户角色', description: '新注册用户的默认角色' }
    ]
  },
  {
    category: 'API 设置',
    items: [
      { name: '格式转换', description: '是否启用跨格式 API 调用' },
      { name: '请求超时', description: '默认的 API 请求超时时间' },
      { name: '流式响应', description: '是否默认启用流式响应' }
    ]
  },
  {
    category: '安全设置',
    items: [
      { name: 'IP 限流', description: '单 IP 的请求频率限制' },
      { name: 'Key 限流', description: '单 Key 的请求频率限制' },
      { name: 'IP 黑名单', description: '全局 IP 黑名单' }
    ]
  }
]
</script>

<template>
  <div class="space-y-8">
    <!-- 标题 -->
    <div class="space-y-4">
      <h1 class="text-3xl font-bold text-[#262624] dark:text-[#f1ead8]">
        高级功能
      </h1>
      <p class="text-lg text-[#666663] dark:text-[#a3a094]">
        {{ siteName }} 提供了一些高级功能，帮助你实现更灵活的 API 管理。
      </p>
    </div>

    <!-- 格式转换 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        格式转换
      </h2>

      <div
        class="p-5"
        :class="[panelClasses.section]"
      >
        <div class="flex items-center gap-3 mb-4">
          <div class="p-2 rounded-lg bg-purple-500/10">
            <Shuffle class="h-5 w-5 text-purple-500" />
          </div>
          <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
            什么是格式转换？
          </h3>
        </div>

        <p class="text-sm text-[#666663] dark:text-[#a3a094] mb-4">
          格式转换允许你用一种 SDK 调用另一种格式的 API。例如用 OpenAI SDK 调用 Claude 模型，
          系统会自动转换请求格式和响应格式。
        </p>

        <div class="grid gap-3 md:grid-cols-3">
          <div
            v-for="example in conversionExamples"
            :key="example.from"
            class="p-4 rounded-lg bg-[#f5f5f0]/50 dark:bg-[#1f1d1a]/50"
          >
            <div class="flex items-center gap-2 mb-2">
              <span :class="panelClasses.badgeBlue">{{ example.from }}</span>
              <ArrowRight class="h-4 w-4 text-[#999]" />
              <span :class="panelClasses.badgeGreen">{{ example.to }}</span>
            </div>
            <p class="text-xs text-[#666663] dark:text-[#a3a094]">
              {{ example.description }}
            </p>
          </div>
        </div>
      </div>

      <!-- 如何启用 -->
      <div
        class="p-5 space-y-4"
        :class="[panelClasses.section]"
      >
        <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
          如何启用格式转换
        </h3>

        <div class="flex items-start gap-4">
          <div class="w-8 h-8 rounded-full bg-[#cc785c] flex items-center justify-center text-white font-bold text-sm flex-shrink-0">
            1
          </div>
          <div>
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              开启系统设置
            </p>
            <p class="text-sm text-[#666663] dark:text-[#a3a094] mt-1">
              在「系统设置」中开启 <code class="text-xs bg-[#f5f5f0] dark:bg-[#1f1d1a] px-1.5 py-0.5 rounded">ENABLE_API_FORMAT_CONVERSION</code>
            </p>
          </div>
        </div>

        <div class="flex items-start gap-4">
          <div class="w-8 h-8 rounded-full bg-[#cc785c] flex items-center justify-center text-white font-bold text-sm flex-shrink-0">
            2
          </div>
          <div>
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              配置端点
            </p>
            <p class="text-sm text-[#666663] dark:text-[#a3a094] mt-1">
              在端点配置中，启用「格式接受配置」并选择接受的入站格式
            </p>
          </div>
        </div>

        <div class="flex items-start gap-4">
          <div class="w-8 h-8 rounded-full bg-[#cc785c] flex items-center justify-center text-white font-bold text-sm flex-shrink-0">
            3
          </div>
          <div>
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              使用
            </p>
            <p class="text-sm text-[#666663] dark:text-[#a3a094] mt-1">
              用户现在可以用 OpenAI SDK 调用配置在 Claude 格式端点上的模型
            </p>
          </div>
        </div>
      </div>

      <div
        class="p-4"
        :class="[panelClasses.section]"
      >
        <div class="flex items-start gap-3">
          <Info class="h-5 w-5 text-blue-500 flex-shrink-0 mt-0.5" />
          <div class="text-sm text-[#666663] dark:text-[#a3a094]">
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              注意事项
            </p>
            <p class="mt-1">
              格式转换会增加少量延迟（通常 &lt;10ms）。部分特有功能（如 Claude 的 thinking、OpenAI 的 function calling）
              可能无法完美转换，建议在实际场景中测试。
            </p>
          </div>
        </div>
      </div>
    </section>

    <!-- 请求头规则 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        请求头规则
      </h2>

      <div
        class="p-5"
        :class="[panelClasses.section]"
      >
        <div class="flex items-center gap-3 mb-4">
          <div class="p-2 rounded-lg bg-green-500/10">
            <FileCode class="h-5 w-5 text-green-500" />
          </div>
          <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
            什么是请求头规则？
          </h3>
        </div>

        <p class="text-sm text-[#666663] dark:text-[#a3a094] mb-4">
          请求头规则允许你在转发请求时修改 HTTP 请求头。可以添加、修改或删除特定的请求头。
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
                    类型
                  </th>
                  <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                    说明
                  </th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="rule in headerRuleTypes"
                  :key="rule.type"
                  class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)] last:border-0"
                >
                  <td class="px-4 py-3 font-medium text-[#262624] dark:text-[#f1ead8]">
                    <span :class="panelClasses.badgeBlue">{{ rule.name }}</span>
                  </td>
                  <td class="px-4 py-3 text-[#666663] dark:text-[#a3a094]">
                    {{ rule.description }}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>

      <div
        class="p-4"
        :class="[panelClasses.section]"
      >
        <div class="flex items-start gap-3">
          <Info class="h-5 w-5 text-blue-500 flex-shrink-0 mt-0.5" />
          <div class="text-sm text-[#666663] dark:text-[#a3a094]">
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              使用场景
            </p>
            <ul class="mt-1 space-y-1">
              <li>• 添加额外的认证信息（如 API 版本号）</li>
              <li>• 添加跟踪标记（如请求 ID、来源标识）</li>
              <li>• 删除敏感信息（如用户原始 IP）</li>
            </ul>
          </div>
        </div>
      </div>
    </section>

    <!-- 请求体规则 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        请求体规则
      </h2>

      <!-- 概述 -->
      <div
        class="p-5"
        :class="[panelClasses.section]"
      >
        <div class="flex items-center gap-3 mb-4">
          <div class="p-2 rounded-lg bg-orange-500/10">
            <FileCode class="h-5 w-5 text-orange-500" />
          </div>
          <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
            什么是请求体规则？
          </h3>
        </div>

        <p class="text-sm text-[#666663] dark:text-[#a3a094] mb-4">
          请求体规则允许你在转发请求时修改请求体（JSON Body）的内容。可以覆写字段、删除字段、向数组追加/插入元素，甚至用正则替换字符串值。
          规则按顺序依次执行，受保护的顶层字段（<code class="text-xs bg-[#f5f5f0] dark:bg-[#1f1d1a] px-1.5 py-0.5 rounded">model</code>、<code class="text-xs bg-[#f5f5f0] dark:bg-[#1f1d1a] px-1.5 py-0.5 rounded">stream</code>）不可修改。
        </p>

        <!-- 操作类型表格 -->
        <div
          class="overflow-hidden"
          :class="[panelClasses.section]"
        >
          <div class="overflow-x-auto">
            <table class="w-full text-sm">
              <thead>
                <tr class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.12)] bg-[#fafaf7]/50 dark:bg-[#1f1d1a]/50">
                  <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                    操作
                  </th>
                  <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                    说明
                  </th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="rule in bodyRuleTypes"
                  :key="rule.action"
                  class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)] last:border-0"
                >
                  <td class="px-4 py-3 font-medium text-[#262624] dark:text-[#f1ead8]">
                    <span :class="panelClasses.badge">{{ rule.name }}</span>
                  </td>
                  <td class="px-4 py-3 text-[#666663] dark:text-[#a3a094]">
                    {{ rule.description }}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>

      <!-- 路径语法 -->
      <div
        class="p-5 space-y-4"
        :class="[panelClasses.section]"
      >
        <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
          路径语法
        </h3>
        <p class="text-sm text-[#666663] dark:text-[#a3a094]">
          使用点号（<code class="text-xs bg-[#f5f5f0] dark:bg-[#1f1d1a] px-1.5 py-0.5 rounded">.</code>）分隔层级，方括号（<code class="text-xs bg-[#f5f5f0] dark:bg-[#1f1d1a] px-1.5 py-0.5 rounded">[N]</code>）访问数组元素。
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
                    路径
                  </th>
                  <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                    说明
                  </th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="ex in pathSyntaxExamples"
                  :key="ex.path"
                  class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)] last:border-0"
                >
                  <td class="px-4 py-3 font-mono text-xs text-[#262624] dark:text-[#f1ead8]">
                    {{ ex.path }}
                  </td>
                  <td class="px-4 py-3 text-[#666663] dark:text-[#a3a094]">
                    {{ ex.desc }}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>

      <!-- 实战示例 -->
      <div
        class="p-5 space-y-4"
        :class="[panelClasses.section]"
      >
        <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
          实战示例
        </h3>

        <div class="grid gap-4 md:grid-cols-2">
          <div
            v-for="example in bodyRuleExamples"
            :key="example.title"
            class="rounded-lg border border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)] overflow-hidden"
          >
            <div class="px-4 py-2.5 border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)] bg-[#fafaf7]/50 dark:bg-[#1f1d1a]/50">
              <p class="font-medium text-sm text-[#262624] dark:text-[#f1ead8]">
                {{ example.title }}
              </p>
              <p class="text-xs text-[#666663] dark:text-[#a3a094] mt-0.5">
                {{ example.description }}
              </p>
            </div>
            <pre class="p-4 text-xs font-mono text-[#262624] dark:text-[#f1ead8] overflow-x-auto bg-[#fafaf7]/30 dark:bg-[#1a1816]/30"><code>{{ example.rule }}</code></pre>
          </div>
        </div>
      </div>

      <!-- 正则替换说明 -->
      <div
        class="p-5 space-y-4"
        :class="[panelClasses.section]"
      >
        <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
          正则替换详解
        </h3>

        <p class="text-sm text-[#666663] dark:text-[#a3a094]">
          <code class="text-xs bg-[#f5f5f0] dark:bg-[#1f1d1a] px-1.5 py-0.5 rounded">regex_replace</code> 对指定路径的字符串值执行正则表达式替换。
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
                    参数
                  </th>
                  <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                    必填
                  </th>
                  <th class="px-4 py-3 text-left font-medium text-[#666663] dark:text-[#a3a094]">
                    说明
                  </th>
                </tr>
              </thead>
              <tbody>
                <tr class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)]">
                  <td class="px-4 py-3 font-mono text-xs text-[#262624] dark:text-[#f1ead8]">
                    path
                  </td>
                  <td class="px-4 py-3">
                    <Check class="h-4 w-4 text-green-500" />
                  </td>
                  <td class="px-4 py-3 text-[#666663] dark:text-[#a3a094]">
                    目标字符串字段的路径
                  </td>
                </tr>
                <tr class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)]">
                  <td class="px-4 py-3 font-mono text-xs text-[#262624] dark:text-[#f1ead8]">
                    pattern
                  </td>
                  <td class="px-4 py-3">
                    <Check class="h-4 w-4 text-green-500" />
                  </td>
                  <td class="px-4 py-3 text-[#666663] dark:text-[#a3a094]">
                    正则表达式（Python re 语法），保存时会校验合法性
                  </td>
                </tr>
                <tr class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)]">
                  <td class="px-4 py-3 font-mono text-xs text-[#262624] dark:text-[#f1ead8]">
                    replacement
                  </td>
                  <td class="px-4 py-3">
                    <Check class="h-4 w-4 text-green-500" />
                  </td>
                  <td class="px-4 py-3 text-[#666663] dark:text-[#a3a094]">
                    替换字符串，留空则删除匹配内容
                  </td>
                </tr>
                <tr class="border-b border-[#e5e4df] dark:border-[rgba(227,224,211,0.08)]">
                  <td class="px-4 py-3 font-mono text-xs text-[#262624] dark:text-[#f1ead8]">
                    flags
                  </td>
                  <td class="px-4 py-3 text-xs text-[#999]">
                    可选
                  </td>
                  <td class="px-4 py-3 text-[#666663] dark:text-[#a3a094]">
                    <code class="text-xs bg-[#f5f5f0] dark:bg-[#1f1d1a] px-1 py-0.5 rounded">i</code> 忽略大小写 /
                    <code class="text-xs bg-[#f5f5f0] dark:bg-[#1f1d1a] px-1 py-0.5 rounded">m</code> 多行模式 /
                    <code class="text-xs bg-[#f5f5f0] dark:bg-[#1f1d1a] px-1 py-0.5 rounded">s</code> dotall（. 匹配换行）
                  </td>
                </tr>
                <tr class="last:border-0">
                  <td class="px-4 py-3 font-mono text-xs text-[#262624] dark:text-[#f1ead8]">
                    count
                  </td>
                  <td class="px-4 py-3 text-xs text-[#999]">
                    可选
                  </td>
                  <td class="px-4 py-3 text-[#666663] dark:text-[#a3a094]">
                    替换次数，默认 0 = 全部替换
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>

      <!-- 使用场景 -->
      <div
        class="p-4"
        :class="[panelClasses.section]"
      >
        <div class="flex items-start gap-3">
          <Info class="h-5 w-5 text-blue-500 flex-shrink-0 mt-0.5" />
          <div class="text-sm text-[#666663] dark:text-[#a3a094]">
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              典型使用场景
            </p>
            <ul class="mt-1 space-y-1">
              <li>
                <span class="font-medium text-[#262624] dark:text-[#f1ead8]">注入 System Prompt</span> — 在所有请求前插入统一的系统提示词
              </li>
              <li>
                <span class="font-medium text-[#262624] dark:text-[#f1ead8]">请求增强</span> — 自动追加上下文、metadata 等字段
              </li>
              <li>
                <span class="font-medium text-[#262624] dark:text-[#f1ead8]">内容过滤</span> — 用正则替换脱敏敏感信息（手机号、邮箱等）
              </li>
              <li>
                <span class="font-medium text-[#262624] dark:text-[#f1ead8]">字段清理</span> — 删除不需要的自定义字段，避免上游报错
              </li>
              <li>
                <span class="font-medium text-[#262624] dark:text-[#f1ead8]">字段适配</span> — 将客户端的字段名重命名为上游期望的格式
              </li>
            </ul>
          </div>
        </div>
      </div>
    </section>

    <!-- 代理设置 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        代理设置
      </h2>

      <div
        class="p-5"
        :class="[panelClasses.section]"
      >
        <div class="flex items-center gap-3 mb-4">
          <div class="p-2 rounded-lg bg-blue-500/10">
            <Globe class="h-5 w-5 text-blue-500" />
          </div>
          <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
            HTTP 代理
          </h3>
        </div>

        <p class="text-sm text-[#666663] dark:text-[#a3a094] mb-4">
          如果你的服务器无法直接访问某些 API（如在国内访问 OpenAI），可以在端点配置中设置代理。
        </p>

        <div class="space-y-3">
          <div class="p-4 rounded-lg bg-[#f5f5f0]/50 dark:bg-[#1f1d1a]/50">
            <h4 class="font-medium text-[#262624] dark:text-[#f1ead8]">
              端点级代理
            </h4>
            <p class="text-sm text-[#666663] dark:text-[#a3a094] mt-1">
              在端点配置中填写代理地址，格式如 <code class="text-xs bg-[#f5f5f0] dark:bg-[#1f1d1a] px-1.5 py-0.5 rounded">http://proxy:8080</code>
            </p>
          </div>

          <div class="p-4 rounded-lg bg-[#f5f5f0]/50 dark:bg-[#1f1d1a]/50">
            <h4 class="font-medium text-[#262624] dark:text-[#f1ead8]">
              全局代理
            </h4>
            <p class="text-sm text-[#666663] dark:text-[#a3a094] mt-1">
              也可以通过环境变量 <code class="text-xs bg-[#f5f5f0] dark:bg-[#1f1d1a] px-1.5 py-0.5 rounded">HTTP_PROXY</code> 设置全局代理
            </p>
          </div>
        </div>
      </div>
    </section>

    <!-- 系统设置概览 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        系统设置
      </h2>

      <p class="text-[#666663] dark:text-[#a3a094]">
        在「系统设置」页面可以配置全局参数：
      </p>

      <div class="grid gap-4 md:grid-cols-3">
        <div
          v-for="category in systemSettings"
          :key="category.category"
          class="p-4"
          :class="[panelClasses.section]"
        >
          <div class="flex items-center gap-2 mb-3">
            <Settings class="h-4 w-4 text-[#cc785c]" />
            <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
              {{ category.category }}
            </h3>
          </div>
          <ul class="space-y-2">
            <li
              v-for="item in category.items"
              :key="item.name"
              class="text-sm"
            >
              <span class="font-medium text-[#262624] dark:text-[#f1ead8]">{{ item.name }}</span>
              <span class="text-[#666663] dark:text-[#a3a094]"> - {{ item.description }}</span>
            </li>
          </ul>
        </div>
      </div>
    </section>

    <!-- 健康监控 -->
    <section class="space-y-4">
      <h2 class="text-xl font-semibold text-[#262624] dark:text-[#f1ead8]">
        健康监控
      </h2>

      <div
        class="p-5"
        :class="[panelClasses.section]"
      >
        <div class="flex items-center gap-3 mb-4">
          <div class="p-2 rounded-lg bg-green-500/10">
            <Shield class="h-5 w-5 text-green-500" />
          </div>
          <h3 class="font-semibold text-[#262624] dark:text-[#f1ead8]">
            端点健康检查
          </h3>
        </div>

        <ul class="space-y-3 text-sm text-[#666663] dark:text-[#a3a094]">
          <li class="flex items-start gap-2">
            <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
            <span>系统会定期检测端点的可用性</span>
          </li>
          <li class="flex items-start gap-2">
            <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
            <span>不健康的端点会被自动跳过，请求会路由到其他可用端点</span>
          </li>
          <li class="flex items-start gap-2">
            <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
            <span>可以在「健康监控」页面查看所有端点的实时状态</span>
          </li>
          <li class="flex items-start gap-2">
            <Check class="h-4 w-4 text-green-500 mt-0.5 flex-shrink-0" />
            <span>支持手动触发健康检查</span>
          </li>
        </ul>
      </div>

      <div
        class="p-4"
        :class="[panelClasses.section]"
      >
        <div class="flex items-start gap-3">
          <AlertTriangle class="h-5 w-5 text-yellow-500 flex-shrink-0 mt-0.5" />
          <div class="text-sm text-[#666663] dark:text-[#a3a094]">
            <p class="font-medium text-[#262624] dark:text-[#f1ead8]">
              端点显示不健康？
            </p>
            <p class="mt-1">
              检查：1) API URL 是否正确；2) API Key 是否有效；3) 网络是否可达；4) 是否需要配置代理。
            </p>
          </div>
        </div>
      </div>
    </section>

    <!-- 下一步 -->
    <section class="pt-4">
      <RouterLink
        to="/guide/faq"
        class="p-4 flex items-center gap-3 group"
        :class="[panelClasses.section, panelClasses.cardHover]"
      >
        <div class="flex-1">
          <div class="font-medium text-[#262624] dark:text-[#f1ead8]">
            常见问题
          </div>
          <div class="text-sm text-[#666663] dark:text-[#a3a094]">
            查看使用中的常见问题和解答
          </div>
        </div>
        <ArrowRight class="h-5 w-5 text-[#999] group-hover:text-[#cc785c] transition-colors" />
      </RouterLink>
    </section>
  </div>
</template>
