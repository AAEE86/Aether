<template>
  <PageContainer>
    <div class="relative flex gap-6">
      <!-- 左侧悬浮目录 -->
      <nav class="hidden lg:block w-44 shrink-0">
        <div class="sticky top-20 pt-20">
          <ul class="space-y-0.5 text-sm border-l border-border">
            <li
              v-for="item in tocItems"
              :key="item.id"
            >
              <button
                class="block w-full text-left px-3 py-1.5 transition-colors duration-200 -ml-px border-l-2"
                :class="activeSection === item.id
                  ? 'border-primary text-primary font-medium'
                  : 'border-transparent text-muted-foreground hover:text-foreground hover:border-border'"
                @click="scrollToSection(item.id)"
              >
                {{ item.label }}
              </button>
            </li>
          </ul>
        </div>
      </nav>

      <!-- 右侧主内容 -->
      <div class="flex-1 min-w-0">
        <PageHeader
          title="系统设置"
          description="管理系统级别的配置和参数"
        />

        <div class="mt-6 space-y-6">
          <!-- 站点信息 -->
          <CardSection
            id="section-site-info"
            title="站点信息"
            description="自定义站点名称和副标题，影响导航栏、登录页、指南页面和邮件等全站显示"
          >
            <template #actions>
              <Button
                size="sm"
                :disabled="siteInfoLoading || !hasSiteInfoChanges"
                @click="saveSiteInfo"
              >
                {{ siteInfoLoading ? '保存中...' : '保存' }}
              </Button>
            </template>
            <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
              <div>
                <Label
                  for="site-name"
                  class="block text-sm font-medium"
                >
                  站点名称
                </Label>
                <Input
                  id="site-name"
                  v-model="systemConfig.site_name"
                  type="text"
                  placeholder="Aether"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  显示在导航栏、登录页标题和邮件中
                </p>
              </div>
              <div>
                <Label
                  for="site-subtitle"
                  class="block text-sm font-medium"
                >
                  站点副标题
                </Label>
                <Input
                  id="site-subtitle"
                  v-model="systemConfig.site_subtitle"
                  type="text"
                  placeholder="AI Gateway"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  显示在导航栏品牌名称下方
                </p>
              </div>
            </div>
          </CardSection>

          <!-- 配置导出/导入 -->
          <CardSection
            id="section-config-mgmt"
            title="配置管理"
            description="导出或导入提供商和模型配置，便于备份或迁移"
          >
            <div class="flex flex-wrap gap-4">
              <div class="flex-1 min-w-[200px]">
                <p class="text-sm text-muted-foreground mb-3">
                  导出当前所有提供商、端点、API Key 和模型配置到 JSON 文件
                </p>
                <Button
                  variant="outline"
                  :disabled="exportLoading"
                  @click="handleExportConfig"
                >
                  <Download class="w-4 h-4 mr-2" />
                  {{ exportLoading ? '导出中...' : '导出配置' }}
                </Button>
              </div>
              <div class="flex-1 min-w-[200px]">
                <p class="text-sm text-muted-foreground mb-3">
                  从 JSON 文件导入配置，支持跳过、覆盖或报错三种冲突处理模式
                </p>
                <div class="flex items-center gap-2">
                  <input
                    ref="configFileInput"
                    type="file"
                    accept=".json"
                    class="hidden"
                    @change="handleConfigFileSelect"
                  >
                  <Button
                    variant="outline"
                    :disabled="importLoading"
                    @click="triggerConfigFileSelect"
                  >
                    <Upload class="w-4 h-4 mr-2" />
                    {{ importLoading ? '导入中...' : '导入配置' }}
                  </Button>
                </div>
              </div>
            </div>
          </CardSection>

          <!-- 用户数据导出/导入 -->
          <CardSection
            id="section-user-data"
            title="用户数据管理"
            description="导出或导入用户及其 API Keys 数据（不含管理员）"
          >
            <div class="flex flex-wrap gap-4">
              <div class="flex-1 min-w-[200px]">
                <p class="text-sm text-muted-foreground mb-3">
                  导出所有普通用户及其 API Keys 到 JSON 文件
                </p>
                <Button
                  variant="outline"
                  :disabled="exportUsersLoading"
                  @click="handleExportUsers"
                >
                  <Download class="w-4 h-4 mr-2" />
                  {{ exportUsersLoading ? '导出中...' : '导出用户数据' }}
                </Button>
              </div>
              <div class="flex-1 min-w-[200px]">
                <p class="text-sm text-muted-foreground mb-3">
                  从 JSON 文件导入用户数据（需相同 ENCRYPTION_KEY）
                </p>
                <div class="flex items-center gap-2">
                  <input
                    ref="usersFileInput"
                    type="file"
                    accept=".json"
                    class="hidden"
                    @change="handleUsersFileSelect"
                  >
                  <Button
                    variant="outline"
                    :disabled="importUsersLoading"
                    @click="triggerUsersFileSelect"
                  >
                    <Upload class="w-4 h-4 mr-2" />
                    {{ importUsersLoading ? '导入中...' : '导入用户数据' }}
                  </Button>
                </div>
              </div>
            </div>
          </CardSection>

          <!-- 网络代理 -->
          <CardSection
            id="section-proxy"
            title="网络代理"
            description="配置提供商出站请求的默认代理，仅影响大模型 API、余额查询、OAuth 等提供商请求"
          >
            <template #actions>
              <Button
                size="sm"
                :disabled="proxyConfigLoading || !hasProxyConfigChanges"
                @click="saveProxyConfig"
              >
                {{ proxyConfigLoading ? '保存中...' : '保存' }}
              </Button>
            </template>
            <div class="max-w-md">
              <Label class="block text-sm font-medium mb-1">
                默认代理节点
              </Label>
              <Select
                :model-value="systemConfig.system_proxy_node_id || '__direct__'"
                @update:model-value="(v: string) => systemConfig.system_proxy_node_id = v === '__direct__' ? null : v"
              >
                <SelectTrigger>
                  <SelectValue placeholder="直连（不使用代理）" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__direct__">
                    直连（不使用代理）
                  </SelectItem>
                  <SelectItem
                    v-for="node in proxyNodesStore.onlineNodes"
                    :key="node.id"
                    :value="node.id"
                  >
                    {{ node.name }}{{ node.region ? ` · ${node.region}` : '' }} ({{ node.ip }}:{{ node.port }})
                  </SelectItem>
                </SelectContent>
              </Select>
              <p class="mt-1 text-xs text-muted-foreground">
                对未单独配置代理的提供商生效，覆盖大模型 API 请求、余额查询、OAuth 刷新等。不影响系统内部接口。
              </p>
            </div>
          </CardSection>

          <!-- 基础配置 -->
          <CardSection
            id="section-basic"
            title="基础配置"
            description="配置系统默认参数"
          >
            <template #actions>
              <Button
                size="sm"
                :disabled="basicConfigLoading || !hasBasicConfigChanges"
                @click="saveBasicConfig"
              >
                {{ basicConfigLoading ? '保存中...' : '保存' }}
              </Button>
            </template>
            <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
              <div>
                <Label
                  for="default-quota"
                  class="block text-sm font-medium"
                >
                  默认用户配额(美元)
                </Label>
                <Input
                  id="default-quota"
                  v-model.number="systemConfig.default_user_quota_usd"
                  type="number"
                  step="0.01"
                  placeholder="10.00"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  新用户注册时的默认配额
                </p>
              </div>

              <div>
                <Label
                  for="rate-limit"
                  class="block text-sm font-medium"
                >
                  每分钟请求限制
                </Label>
                <Input
                  id="rate-limit"
                  v-model.number="systemConfig.rate_limit_per_minute"
                  type="number"
                  placeholder="0"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  0 表示不限制
                </p>
              </div>

              <div class="flex items-center h-full">
                <div class="flex items-center space-x-2">
                  <Checkbox
                    id="enable-registration"
                    v-model:checked="systemConfig.enable_registration"
                  />
                  <div>
                    <Label
                      for="enable-registration"
                      class="cursor-pointer"
                    >
                      开放用户注册
                    </Label>
                    <p class="text-xs text-muted-foreground">
                      允许新用户自助注册账户
                    </p>
                  </div>
                </div>
              </div>

              <div class="flex items-center h-full">
                <div class="flex items-center space-x-2">
                  <Checkbox
                    id="auto-delete-expired-keys"
                    v-model:checked="systemConfig.auto_delete_expired_keys"
                  />
                  <div>
                    <Label
                      for="auto-delete-expired-keys"
                      class="cursor-pointer"
                    >
                      自动删除过期 Key
                    </Label>
                    <p class="text-xs text-muted-foreground">
                      关闭时仅禁用过期的独立余额 Key
                    </p>
                  </div>
                </div>
              </div>

              <div class="flex items-center h-full">
                <div class="flex items-center space-x-2">
                  <Checkbox
                    id="enable-format-conversion"
                    v-model:checked="systemConfig.enable_format_conversion"
                  />
                  <div>
                    <Label
                      for="enable-format-conversion"
                      class="cursor-pointer"
                    >
                      全局格式转换
                    </Label>
                    <p class="text-xs text-muted-foreground">
                      开启后强制允许所有提供商接受跨格式请求
                    </p>
                  </div>
                </div>
              </div>
            </div>
          </CardSection>

          <!-- 请求记录配置 -->
          <CardSection
            id="section-request-log"
            title="请求记录"
            description="控制请求/响应详情的入库方式和内容"
          >
            <template #actions>
              <Button
                size="sm"
                :disabled="logConfigLoading || !hasLogConfigChanges"
                @click="saveLogConfig"
              >
                {{ logConfigLoading ? '保存中...' : '保存' }}
              </Button>
            </template>
            <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
              <div>
                <Label
                  for="request-log-level"
                  class="block text-sm font-medium mb-2"
                >
                  记录详细程度
                </Label>
                <Select
                  v-model="systemConfig.request_record_level"
                >
                  <SelectTrigger
                    id="request-log-level"
                    class="mt-1"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="basic">
                      BASIC - 基本信息 (~1KB/条)
                    </SelectItem>
                    <SelectItem value="headers">
                      HEADERS - 含请求头 (~2-3KB/条)
                    </SelectItem>
                    <SelectItem value="full">
                      FULL - 完整请求响应 (~50KB/条)
                    </SelectItem>
                  </SelectContent>
                </Select>
                <p class="mt-1 text-xs text-muted-foreground">
                  敏感信息会自动脱敏
                </p>
              </div>

              <div>
                <Label
                  for="max-request-body-size"
                  class="block text-sm font-medium"
                >
                  最大请求体大小 (KB)
                </Label>
                <Input
                  id="max-request-body-size"
                  v-model.number="maxRequestBodySizeKB"
                  type="number"
                  placeholder="512"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  超过此大小的请求体将被截断记录
                </p>
              </div>

              <div>
                <Label
                  for="max-response-body-size"
                  class="block text-sm font-medium"
                >
                  最大响应体大小 (KB)
                </Label>
                <Input
                  id="max-response-body-size"
                  v-model.number="maxResponseBodySizeKB"
                  type="number"
                  placeholder="512"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  超过此大小的响应体将被截断记录
                </p>
              </div>

              <div>
                <Label
                  for="sensitive-headers"
                  class="block text-sm font-medium"
                >
                  敏感请求头
                </Label>
                <Input
                  id="sensitive-headers"
                  v-model="sensitiveHeadersStr"
                  placeholder="authorization, x-api-key, cookie"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  逗号分隔，这些请求头会被脱敏处理
                </p>
              </div>
            </div>
          </CardSection>

          <!-- 请求记录清理策略 -->
          <CardSection
            id="section-cleanup"
            title="请求记录清理策略"
            description="配置请求记录的分级保留和自动清理"
          >
            <template #actions>
              <div class="flex items-center gap-4">
                <div class="flex items-center gap-2">
                  <Switch
                    id="enable-auto-cleanup"
                    :model-value="systemConfig.enable_auto_cleanup"
                    @update:model-value="handleAutoCleanupToggle"
                  />
                  <div>
                    <Label
                      for="enable-auto-cleanup"
                      class="text-sm cursor-pointer"
                    >
                      启用自动清理
                    </Label>
                    <p class="text-xs text-muted-foreground">
                      每天凌晨执行
                    </p>
                  </div>
                </div>
                <Button
                  size="sm"
                  :disabled="cleanupConfigLoading || !hasCleanupConfigChanges"
                  @click="saveCleanupConfig"
                >
                  {{ cleanupConfigLoading ? '保存中...' : '保存' }}
                </Button>
              </div>
            </template>
            <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
              <div>
                <Label
                  for="detail-log-retention-days"
                  class="block text-sm font-medium"
                >
                  详细记录保留天数
                </Label>
                <Input
                  id="detail-log-retention-days"
                  v-model.number="systemConfig.detail_log_retention_days"
                  type="number"
                  placeholder="7"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  超过后压缩 body 字段
                </p>
              </div>

              <div>
                <Label
                  for="compressed-log-retention-days"
                  class="block text-sm font-medium"
                >
                  压缩记录保留天数
                </Label>
                <Input
                  id="compressed-log-retention-days"
                  v-model.number="systemConfig.compressed_log_retention_days"
                  type="number"
                  placeholder="90"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  超过后删除 body 字段
                </p>
              </div>

              <div>
                <Label
                  for="header-retention-days"
                  class="block text-sm font-medium"
                >
                  请求头保留天数
                </Label>
                <Input
                  id="header-retention-days"
                  v-model.number="systemConfig.header_retention_days"
                  type="number"
                  placeholder="90"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  超过后清空 headers 字段
                </p>
              </div>

              <div>
                <Label
                  for="log-retention-days"
                  class="block text-sm font-medium"
                >
                  完整记录保留天数
                </Label>
                <Input
                  id="log-retention-days"
                  v-model.number="systemConfig.log_retention_days"
                  type="number"
                  placeholder="365"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  超过后删除整条记录
                </p>
              </div>

              <div>
                <Label
                  for="cleanup-batch-size"
                  class="block text-sm font-medium"
                >
                  每批次清理记录数
                </Label>
                <Input
                  id="cleanup-batch-size"
                  v-model.number="systemConfig.cleanup_batch_size"
                  type="number"
                  placeholder="1000"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  避免单次操作过大影响性能
                </p>
              </div>

              <div>
                <Label
                  for="audit-log-retention-days"
                  class="block text-sm font-medium"
                >
                  审计日志保留天数
                </Label>
                <Input
                  id="audit-log-retention-days"
                  v-model.number="systemConfig.audit_log_retention_days"
                  type="number"
                  placeholder="30"
                  class="mt-1"
                />
                <p class="mt-1 text-xs text-muted-foreground">
                  超过后删除审计日志记录
                </p>
              </div>
            </div>

            <!-- 清理策略说明 -->
            <div class="mt-4 p-4 bg-muted/50 rounded-lg">
              <h4 class="text-sm font-medium mb-2">
                清理策略说明
              </h4>
              <div class="text-xs text-muted-foreground space-y-1">
                <p>1. <strong>详细日志阶段</strong>: 保留完整的 request_body 和 response_body</p>
                <p>2. <strong>压缩日志阶段</strong>: body 字段被压缩存储，节省空间</p>
                <p>3. <strong>统计阶段</strong>: 仅保留 tokens、成本等统计信息</p>
                <p>4. <strong>归档删除</strong>: 超过保留期限后完全删除记录</p>
                <p>5. <strong>审计日志</strong>: 独立清理，记录用户登录、操作等安全事件</p>
              </div>
            </div>
          </CardSection>

          <!-- 定时任务 -->
          <CardSection
            id="section-scheduled"
            title="定时任务"
            description="配置系统后台定时任务"
          >
            <div class="space-y-3">
              <template
                v-for="task in scheduledTasks"
                :key="task.id"
              >
                <div
                  class="group relative rounded-xl border transition-all duration-300"
                  :class="task.enabled
                    ? 'border-primary/30 bg-primary/[0.02] shadow-sm shadow-primary/5'
                    : 'border-border bg-card hover:border-border/80'"
                >
                  <!-- 主行 -->
                  <div class="flex items-center gap-4 p-4">
                    <!-- 左侧：开关 -->
                    <div class="shrink-0">
                      <Switch
                        :id="`enable-${task.id}`"
                        :model-value="task.enabled"
                        @update:model-value="task.onToggle"
                      />
                    </div>

                    <!-- 中间：图标、标题、描述 -->
                    <div class="flex items-center gap-3 flex-1 min-w-0">
                      <div
                        class="w-9 h-9 rounded-lg flex items-center justify-center shrink-0 transition-colors duration-300"
                        :class="task.enabled
                          ? 'bg-primary/10 text-primary'
                          : 'text-muted-foreground'"
                      >
                        <component
                          :is="task.icon"
                          class="w-4.5 h-4.5"
                        />
                      </div>
                      <div class="flex-1 min-w-0">
                        <h4 class="font-medium text-sm">
                          {{ task.title }}
                        </h4>
                        <p class="text-xs text-muted-foreground mt-0.5 truncate">
                          {{ task.description }}
                        </p>
                      </div>
                    </div>

                    <!-- 右侧：时间选择器 + 保存按钮 -->
                    <div
                      v-if="task.enabled && task.hasTimeConfig"
                      class="flex items-center gap-2 shrink-0"
                    >
                      <Clock class="w-4 h-4 text-muted-foreground" />
                      <Select
                        :model-value="task.hour"
                        @update:model-value="(val: string) => task.updateTime(val, task.minute)"
                      >
                        <SelectTrigger class="w-[4.5rem] h-8 text-xs">
                          <SelectValue placeholder="时" />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem
                            v-for="h in 24"
                            :key="h - 1"
                            :value="String(h - 1).padStart(2, '0')"
                          >
                            {{ String(h - 1).padStart(2, '0') }}
                          </SelectItem>
                        </SelectContent>
                      </Select>
                      <span class="text-sm text-muted-foreground">:</span>
                      <Select
                        :model-value="task.minute"
                        @update:model-value="(val: string) => task.updateTime(task.hour, val)"
                      >
                        <SelectTrigger class="w-[4.5rem] h-8 text-xs">
                          <SelectValue placeholder="分" />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem
                            v-for="m in 60"
                            :key="m - 1"
                            :value="String(m - 1).padStart(2, '0')"
                          >
                            {{ String(m - 1).padStart(2, '0') }}
                          </SelectItem>
                        </SelectContent>
                      </Select>
                      <Button
                        v-if="task.hasChanges"
                        variant="default"
                        size="sm"
                        class="h-8 px-2.5 text-xs"
                        :disabled="task.loading"
                        @click="task.onSave"
                      >
                        <Check
                          v-if="!task.loading"
                          class="w-3.5 h-3.5"
                        />
                        <Loader2
                          v-else
                          class="w-3.5 h-3.5 animate-spin"
                        />
                      </Button>
                    </div>
                  </div>

                  <!-- 额外配置区域（仅用户配额重置任务有） -->
                  <div
                    v-if="task.id === 'user-quota-reset' && task.enabled"
                    class="px-4 pb-4 pt-0"
                  >
                    <div class="flex items-center gap-3 p-3 rounded-lg bg-muted/30 border border-border/50">
                      <div class="flex items-center gap-2 text-sm">
                        <span class="text-muted-foreground">重置周期</span>
                        <div class="flex items-center gap-1.5">
                          <span class="text-muted-foreground">每</span>
                          <Input
                            v-model.number="systemConfig.user_quota_reset_interval_days"
                            type="number"
                            min="1"
                            step="1"
                            class="w-14 h-7 text-xs text-center px-2"
                          />
                          <span class="text-muted-foreground">天</span>
                        </div>
                      </div>
                    </div>
                    <p class="text-[11px] text-muted-foreground mt-2 ml-1">
                      滚动计算：距离上次成功执行满 N 天后再次执行
                    </p>
                  </div>
                </div>
              </template>
            </div>
          </CardSection>

          <!-- 系统版本信息 -->
          <CardSection
            id="section-sysinfo"
            title="系统信息"
            description="当前系统版本和构建信息"
          >
            <div class="flex items-center gap-4">
              <div class="flex items-center gap-2">
                <Label class="text-sm font-medium text-muted-foreground">版本:</Label>
                <span
                  v-if="systemVersion"
                  class="text-sm font-mono"
                >
                  {{ systemVersion }}
                </span>
                <span
                  v-else
                  class="text-sm text-muted-foreground"
                >
                  加载中...
                </span>
              </div>
            </div>
          </CardSection>
        </div>
      </div>
    </div>

    <!-- 导入配置对话框 -->
    <Dialog
      v-model:open="importDialogOpen"
      title="导入配置"
      description="选择冲突处理模式并确认导入"
    >
      <div class="space-y-4">
        <div
          v-if="importPreview"
          class="text-sm"
        >
          <p class="font-medium mb-2">
            配置预览
          </p>
          <ul class="space-y-1 text-muted-foreground">
            <li>全局模型: {{ importPreview.global_models?.length || 0 }} 个</li>
            <li>提供商: {{ importPreview.providers?.length || 0 }} 个</li>
            <li>
              端点: {{ importPreview.providers?.reduce((sum: number, p: any) => sum + (p.endpoints?.length || 0), 0) }} 个
            </li>
            <li>
              API Keys: {{ importPreview.providers?.reduce((sum: number, p: any) => sum + (p.api_keys?.length || 0), 0) }} 个
            </li>
            <li v-if="importPreview.ldap_config">
              LDAP 配置: 1 个
            </li>
            <li v-if="importPreview.oauth_providers?.length">
              OAuth Providers: {{ importPreview.oauth_providers.length }} 个
            </li>
          </ul>
        </div>

        <div>
          <Label class="block text-sm font-medium mb-2">冲突处理模式</Label>
          <Select
            v-model="mergeMode"
            v-model:open="mergeModeSelectOpen"
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="skip">
                跳过 - 保留现有配置
              </SelectItem>
              <SelectItem value="overwrite">
                覆盖 - 用导入配置替换
              </SelectItem>
              <SelectItem value="error">
                报错 - 遇到冲突时中止
              </SelectItem>
            </SelectContent>
          </Select>
          <p class="mt-1 text-xs text-muted-foreground">
            <template v-if="mergeMode === 'skip'">
              已存在的配置将被保留，仅导入新配置
            </template>
            <template v-else-if="mergeMode === 'overwrite'">
              已存在的配置将被导入的配置覆盖
            </template>
            <template v-else>
              如果发现任何冲突，导入将中止并回滚
            </template>
          </p>
        </div>

        <p class="text-xs text-muted-foreground">
          注意：相同的 API Keys 会自动跳过，不会创建重复记录。
        </p>
      </div>

      <template #footer>
        <Button
          variant="outline"
          @click="importDialogOpen = false; mergeModeSelectOpen = false"
        >
          取消
        </Button>
        <Button
          :disabled="importLoading"
          @click="confirmImport"
        >
          {{ importLoading ? '导入中...' : '确认导入' }}
        </Button>
      </template>
    </Dialog>

    <!-- 导入结果对话框 -->
    <Dialog
      v-model:open="importResultDialogOpen"
      title="导入完成"
    >
      <div
        v-if="importResult"
        class="space-y-4"
      >
        <div class="grid grid-cols-2 gap-4 text-sm">
          <div>
            <p class="font-medium">
              全局模型
            </p>
            <p class="text-muted-foreground">
              创建: {{ importResult.stats.global_models.created }},
              更新: {{ importResult.stats.global_models.updated }},
              跳过: {{ importResult.stats.global_models.skipped }}
            </p>
          </div>
          <div>
            <p class="font-medium">
              提供商
            </p>
            <p class="text-muted-foreground">
              创建: {{ importResult.stats.providers.created }},
              更新: {{ importResult.stats.providers.updated }},
              跳过: {{ importResult.stats.providers.skipped }}
            </p>
          </div>
          <div>
            <p class="font-medium">
              端点
            </p>
            <p class="text-muted-foreground">
              创建: {{ importResult.stats.endpoints.created }},
              更新: {{ importResult.stats.endpoints.updated }},
              跳过: {{ importResult.stats.endpoints.skipped }}
            </p>
          </div>
          <div>
            <p class="font-medium">
              API Keys
            </p>
            <p class="text-muted-foreground">
              创建: {{ importResult.stats.keys.created }},
              跳过: {{ importResult.stats.keys.skipped }}
            </p>
          </div>
          <div class="col-span-2">
            <p class="font-medium">
              模型配置
            </p>
            <p class="text-muted-foreground">
              创建: {{ importResult.stats.models.created }},
              更新: {{ importResult.stats.models.updated }},
              跳过: {{ importResult.stats.models.skipped }}
            </p>
          </div>
          <div v-if="importResult.stats.ldap">
            <p class="font-medium">
              LDAP 配置
            </p>
            <p class="text-muted-foreground">
              创建: {{ importResult.stats.ldap.created }},
              更新: {{ importResult.stats.ldap.updated }},
              跳过: {{ importResult.stats.ldap.skipped }}
            </p>
          </div>
          <div v-if="importResult.stats.oauth">
            <p class="font-medium">
              OAuth Providers
            </p>
            <p class="text-muted-foreground">
              创建: {{ importResult.stats.oauth.created }},
              更新: {{ importResult.stats.oauth.updated }},
              跳过: {{ importResult.stats.oauth.skipped }}
            </p>
          </div>
        </div>

        <div
          v-if="importResult.stats.errors.length > 0"
          class="p-3 bg-destructive/10 rounded-lg"
        >
          <p class="font-medium text-destructive mb-2">
            警告信息
          </p>
          <ul class="text-sm text-destructive space-y-1">
            <li
              v-for="(err, index) in importResult.stats.errors"
              :key="index"
            >
              {{ err }}
            </li>
          </ul>
        </div>
      </div>

      <template #footer>
        <Button @click="importResultDialogOpen = false">
          确定
        </Button>
      </template>
    </Dialog>

    <!-- 用户数据导入对话框 -->
    <Dialog
      v-model:open="importUsersDialogOpen"
      title="导入用户数据"
      description="选择冲突处理模式并确认导入"
    >
      <div class="space-y-4">
        <div
          v-if="importUsersPreview"
          class="text-sm"
        >
          <p class="font-medium mb-2">
            数据预览
          </p>
          <ul class="space-y-1 text-muted-foreground">
            <li>用户: {{ importUsersPreview.users?.length || 0 }} 个</li>
            <li>
              API Keys: {{ importUsersPreview.users?.reduce((sum: number, u: any) => sum + (u.api_keys?.length || 0), 0) }} 个
            </li>
            <li v-if="importUsersPreview.standalone_keys?.length">
              独立余额 Keys: {{ importUsersPreview.standalone_keys.length }} 个
            </li>
          </ul>
        </div>

        <div>
          <Label class="block text-sm font-medium mb-2">冲突处理模式</Label>
          <Select
            v-model="usersMergeMode"
            v-model:open="usersMergeModeSelectOpen"
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="skip">
                跳过 - 保留现有用户
              </SelectItem>
              <SelectItem value="overwrite">
                覆盖 - 用导入数据替换
              </SelectItem>
              <SelectItem value="error">
                报错 - 遇到冲突时中止
              </SelectItem>
            </SelectContent>
          </Select>
          <p class="mt-1 text-xs text-muted-foreground">
            <template v-if="usersMergeMode === 'skip'">
              已存在的用户将被保留，仅导入新用户
            </template>
            <template v-else-if="usersMergeMode === 'overwrite'">
              已存在的用户将被导入的数据覆盖
            </template>
            <template v-else>
              如果发现任何冲突，导入将中止并回滚
            </template>
          </p>
        </div>

        <p class="text-xs text-muted-foreground">
          注意：用户 API Keys 需要目标系统使用相同的 ENCRYPTION_KEY 环境变量才能正常工作。
        </p>
      </div>

      <template #footer>
        <Button
          variant="outline"
          @click="importUsersDialogOpen = false; usersMergeModeSelectOpen = false"
        >
          取消
        </Button>
        <Button
          :disabled="importUsersLoading"
          @click="confirmImportUsers"
        >
          {{ importUsersLoading ? '导入中...' : '确认导入' }}
        </Button>
      </template>
    </Dialog>

    <!-- 用户数据导入结果对话框 -->
    <Dialog
      v-model:open="importUsersResultDialogOpen"
      title="用户数据导入完成"
    >
      <div
        v-if="importUsersResult"
        class="space-y-4"
      >
        <div class="grid grid-cols-2 gap-4 text-sm">
          <div>
            <p class="font-medium">
              用户
            </p>
            <p class="text-muted-foreground">
              创建: {{ importUsersResult.stats.users.created }},
              更新: {{ importUsersResult.stats.users.updated }},
              跳过: {{ importUsersResult.stats.users.skipped }}
            </p>
          </div>
          <div>
            <p class="font-medium">
              API Keys
            </p>
            <p class="text-muted-foreground">
              创建: {{ importUsersResult.stats.api_keys.created }},
              跳过: {{ importUsersResult.stats.api_keys.skipped }}
            </p>
          </div>
          <div
            v-if="importUsersResult.stats.standalone_keys"
            class="col-span-2"
          >
            <p class="font-medium">
              独立余额 Keys
            </p>
            <p class="text-muted-foreground">
              创建: {{ importUsersResult.stats.standalone_keys.created }},
              跳过: {{ importUsersResult.stats.standalone_keys.skipped }}
            </p>
          </div>
        </div>

        <div
          v-if="importUsersResult.stats.errors.length > 0"
          class="p-3 bg-destructive/10 rounded-lg"
        >
          <p class="font-medium text-destructive mb-2">
            警告信息
          </p>
          <ul class="text-sm text-destructive space-y-1">
            <li
              v-for="(err, index) in importUsersResult.stats.errors"
              :key="index"
            >
              {{ err }}
            </li>
          </ul>
        </div>
      </div>

      <template #footer>
        <Button @click="importUsersResultDialogOpen = false">
          确定
        </Button>
      </template>
    </Dialog>
  </PageContainer>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { Download, Upload, CalendarCheck, RotateCcw, RefreshCw, Clock, Check, Loader2 } from 'lucide-vue-next'
import Button from '@/components/ui/button.vue'
import Input from '@/components/ui/input.vue'
import Label from '@/components/ui/label.vue'
import Checkbox from '@/components/ui/checkbox.vue'
import Switch from '@/components/ui/switch.vue'
import Select from '@/components/ui/select.vue'
import SelectTrigger from '@/components/ui/select-trigger.vue'
import SelectValue from '@/components/ui/select-value.vue'
import SelectContent from '@/components/ui/select-content.vue'
import SelectItem from '@/components/ui/select-item.vue'
import {
  Dialog,
} from '@/components/ui'
import { PageHeader, PageContainer, CardSection } from '@/components/layout'
import { useToast } from '@/composables/useToast'
import { adminApi, type ConfigExportData, type ConfigImportResponse, type UsersExportData, type UsersImportResponse } from '@/api/admin'
import { log } from '@/utils/logger'
import { useProxyNodesStore } from '@/stores/proxy-nodes'
import { useSiteInfo } from '@/composables/useSiteInfo'

const { success, error } = useToast()
const { refreshSiteInfo } = useSiteInfo()
const proxyNodesStore = useProxyNodesStore()

// 目录项定义
const tocItems = [
  { id: 'section-site-info', label: '站点信息' },
  { id: 'section-config-mgmt', label: '配置管理' },
  { id: 'section-user-data', label: '用户数据管理' },
  { id: 'section-proxy', label: '网络代理' },
  { id: 'section-basic', label: '基础配置' },
  { id: 'section-request-log', label: '请求记录' },
  { id: 'section-cleanup', label: '记录清理策略' },
  { id: 'section-scheduled', label: '定时任务' },
  { id: 'section-sysinfo', label: '系统信息' },
]

const activeSection = ref(tocItems[0].id)
let observer: IntersectionObserver | null = null

function getScrollContainer(): HTMLElement | null {
  return document.querySelector('.app-shell__content')
}

function scrollToSection(id: string) {
  const el = document.getElementById(id)
  const container = getScrollContainer()
  if (el && container) {
    const offset = 80
    const top = el.getBoundingClientRect().top - container.getBoundingClientRect().top + container.scrollTop - offset
    container.scrollTo({ top, behavior: 'smooth' })
  }
}

function setupScrollSpy() {
  const sectionIds = tocItems.map(item => item.id)
  const container = getScrollContainer()
  if (!container) return

  const visibleSections = new Set<string>()

  observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          visibleSections.add(entry.target.id)
        } else {
          visibleSections.delete(entry.target.id)
        }
      }
      // 利用 sectionIds 的有序性，取可见中最靠前的
      const topId = sectionIds.find(id => visibleSections.has(id))
      if (topId) {
        activeSection.value = topId
      }
    },
    { root: container, rootMargin: '-80px 0px -60% 0px', threshold: 0 }
  )

  for (const id of sectionIds) {
    const el = document.getElementById(id)
    if (el) observer.observe(el)
  }
}

interface SystemConfig {
  // 站点信息
  site_name: string
  site_subtitle: string
  // 网络代理
  system_proxy_node_id: string | null
  // 基础配置
  default_user_quota_usd: number
  rate_limit_per_minute: number
  enable_registration: boolean
  // 独立余额 Key 过期管理
  auto_delete_expired_keys: boolean
  // 格式转换
  enable_format_conversion: boolean
  // 请求记录
  request_record_level: string
  max_request_body_size: number
  max_response_body_size: number
  sensitive_headers: string[]
  // 请求记录清理
  enable_auto_cleanup: boolean
  detail_log_retention_days: number
  compressed_log_retention_days: number
  header_retention_days: number
  log_retention_days: number
  cleanup_batch_size: number
  audit_log_retention_days: number
  // 定时任务
  enable_provider_checkin: boolean
  provider_checkin_time: string
  enable_user_quota_reset: boolean
  user_quota_reset_time: string
  user_quota_reset_interval_days: number
  enable_oauth_token_refresh: boolean
}

const siteInfoLoading = ref(false)
const proxyConfigLoading = ref(false)
const basicConfigLoading = ref(false)
const logConfigLoading = ref(false)
const cleanupConfigLoading = ref(false)

// 导出/导入相关
const exportLoading = ref(false)
const importLoading = ref(false)
const importDialogOpen = ref(false)
const importResultDialogOpen = ref(false)
const configFileInput = ref<HTMLInputElement | null>(null)
const importPreview = ref<ConfigExportData | null>(null)
const importResult = ref<ConfigImportResponse | null>(null)
const mergeMode = ref<'skip' | 'overwrite' | 'error'>('skip')
const mergeModeSelectOpen = ref(false)

// 用户数据导出/导入相关
const exportUsersLoading = ref(false)
const importUsersLoading = ref(false)
const importUsersDialogOpen = ref(false)
const importUsersResultDialogOpen = ref(false)
const usersFileInput = ref<HTMLInputElement | null>(null)
const importUsersPreview = ref<UsersExportData | null>(null)
const importUsersResult = ref<UsersImportResponse | null>(null)
const usersMergeMode = ref<'skip' | 'overwrite' | 'error'>('skip')
const usersMergeModeSelectOpen = ref(false)

// 系统版本信息
const systemVersion = ref<string>('')

const systemConfig = ref<SystemConfig>({
  // 站点信息
  site_name: 'Aether',
  site_subtitle: 'AI Gateway',
  // 网络代理
  system_proxy_node_id: null,
  // 基础配置
  default_user_quota_usd: 10.0,
  rate_limit_per_minute: 0,
  enable_registration: false,
  // 独立余额 Key 过期管理
  auto_delete_expired_keys: false,
  // 格式转换
  enable_format_conversion: false,
  // 请求记录
  request_record_level: 'basic',
  max_request_body_size: 1048576,
  max_response_body_size: 1048576,
  sensitive_headers: ['authorization', 'x-api-key', 'api-key', 'cookie', 'set-cookie'],
  // 请求记录清理
  enable_auto_cleanup: true,
  detail_log_retention_days: 7,
  compressed_log_retention_days: 90,
  header_retention_days: 90,
  log_retention_days: 365,
  cleanup_batch_size: 1000,
  audit_log_retention_days: 30,
  // 定时任务
  enable_provider_checkin: true,
  provider_checkin_time: '01:05',
  enable_user_quota_reset: false,
  user_quota_reset_time: '05:00',
  user_quota_reset_interval_days: 1,
  enable_oauth_token_refresh: true,
})

// 原始配置值（用于检测变动）
const originalConfig = ref<SystemConfig | null>(null)

// 检测各模块是否有变动
const hasSiteInfoChanges = computed(() => {
  if (!originalConfig.value) return false
  return (
    systemConfig.value.site_name !== originalConfig.value.site_name ||
    systemConfig.value.site_subtitle !== originalConfig.value.site_subtitle
  )
})

const hasBasicConfigChanges = computed(() => {
  if (!originalConfig.value) return false
  return (
    systemConfig.value.default_user_quota_usd !== originalConfig.value.default_user_quota_usd ||
    systemConfig.value.rate_limit_per_minute !== originalConfig.value.rate_limit_per_minute ||
    systemConfig.value.enable_registration !== originalConfig.value.enable_registration ||
    systemConfig.value.auto_delete_expired_keys !== originalConfig.value.auto_delete_expired_keys ||
    systemConfig.value.enable_format_conversion !== originalConfig.value.enable_format_conversion
  )
})

const hasLogConfigChanges = computed(() => {
  if (!originalConfig.value) return false
  return (
    systemConfig.value.request_record_level !== originalConfig.value.request_record_level ||
    systemConfig.value.max_request_body_size !== originalConfig.value.max_request_body_size ||
    systemConfig.value.max_response_body_size !== originalConfig.value.max_response_body_size ||
    JSON.stringify(systemConfig.value.sensitive_headers) !== JSON.stringify(originalConfig.value.sensitive_headers)
  )
})

const hasCleanupConfigChanges = computed(() => {
  if (!originalConfig.value) return false
  return (
    systemConfig.value.detail_log_retention_days !== originalConfig.value.detail_log_retention_days ||
    systemConfig.value.compressed_log_retention_days !== originalConfig.value.compressed_log_retention_days ||
    systemConfig.value.header_retention_days !== originalConfig.value.header_retention_days ||
    systemConfig.value.log_retention_days !== originalConfig.value.log_retention_days ||
    systemConfig.value.cleanup_batch_size !== originalConfig.value.cleanup_batch_size ||
    systemConfig.value.audit_log_retention_days !== originalConfig.value.audit_log_retention_days
  )
})

// 计算属性：KB 和 字节 之间的转换
const maxRequestBodySizeKB = computed({
  get: () => Math.round(systemConfig.value.max_request_body_size / 1024),
  set: (val: number) => {
    systemConfig.value.max_request_body_size = val * 1024
  }
})

const maxResponseBodySizeKB = computed({
  get: () => Math.round(systemConfig.value.max_response_body_size / 1024),
  set: (val: number) => {
    systemConfig.value.max_response_body_size = val * 1024
  }
})

// 计算属性：敏感请求头数组和字符串之间的转换
const sensitiveHeadersStr = computed({
  get: () => systemConfig.value.sensitive_headers.join(', '),
  set: (val: string) => {
    systemConfig.value.sensitive_headers = val
      .split(',')
      .map(s => s.trim().toLowerCase())
      .filter(s => s.length > 0)
  }
})

onMounted(async () => {
  await Promise.all([
    loadSystemConfig(),
    loadSystemVersion(),
    proxyNodesStore.ensureLoaded(),
  ])
  await nextTick()
  setupScrollSpy()
})

onBeforeUnmount(() => {
  if (observer) {
    observer.disconnect()
    observer = null
  }
})

async function loadSystemVersion() {
  try {
    const data = await adminApi.getSystemVersion()
    systemVersion.value = data.version
  } catch (err) {
    log.error('加载系统版本失败:', err)
  }
}

async function loadSystemConfig() {
  try {
    const configs = [
      // 站点信息
      'site_name',
      'site_subtitle',
      // 网络代理
      'system_proxy_node_id',
      // 基础配置
      'default_user_quota_usd',
      'rate_limit_per_minute',
      'enable_registration',
      // 独立余额 Key 过期管理
      'auto_delete_expired_keys',
      // 格式转换
      'enable_format_conversion',
      // 请求记录
      'request_record_level',
      'max_request_body_size',
      'max_response_body_size',
      'sensitive_headers',
      // 请求记录清理
      'enable_auto_cleanup',
      'detail_log_retention_days',
      'compressed_log_retention_days',
      'header_retention_days',
      'log_retention_days',
      'cleanup_batch_size',
      'audit_log_retention_days',
      // 定时任务
      'enable_provider_checkin',
      'provider_checkin_time',
      'enable_user_quota_reset',
      'user_quota_reset_time',
      'user_quota_reset_interval_days',
      'enable_oauth_token_refresh',
    ]

    for (const key of configs) {
      try {
        const response = await adminApi.getSystemConfig(key)
        if (response.value !== null && response.value !== undefined) {
          (systemConfig.value as any)[key] = response.value
        }
      } catch {
        // 配置不存在时使用默认值，无需处理
      }
    }
    // 保存原始值用于变动检测
    originalConfig.value = JSON.parse(JSON.stringify(systemConfig.value))
    // 初始化签到时间的原始值（用于回滚）
    previousCheckinTime.value = systemConfig.value.provider_checkin_time
    // 初始化配额重置时间的原始值（用于回滚）
    previousUserQuotaResetTime.value = systemConfig.value.user_quota_reset_time
    previousUserQuotaResetIntervalDays.value = systemConfig.value.user_quota_reset_interval_days
  } catch (err) {
    error('加载系统配置失败')
    log.error('加载系统配置失败:', err)
  }
}

const hasProxyConfigChanges = computed(() => {
  if (!originalConfig.value) return false
  return systemConfig.value.system_proxy_node_id !== originalConfig.value.system_proxy_node_id
})

async function saveSiteInfo() {
  siteInfoLoading.value = true
  try {
    const configItems = [
      { key: 'site_name', value: systemConfig.value.site_name, description: '站点名称' },
      { key: 'site_subtitle', value: systemConfig.value.site_subtitle, description: '站点副标题' },
    ]
    await Promise.all(
      configItems.map(item =>
        adminApi.updateSystemConfig(item.key, item.value, item.description)
      )
    )
    if (originalConfig.value) {
      originalConfig.value.site_name = systemConfig.value.site_name
      originalConfig.value.site_subtitle = systemConfig.value.site_subtitle
    }
    await refreshSiteInfo()
    success('站点信息已保存')
  } catch (err) {
    error('保存站点信息失败')
    log.error('保存站点信息失败:', err)
  } finally {
    siteInfoLoading.value = false
  }
}

async function saveProxyConfig() {
  proxyConfigLoading.value = true
  try {
    await adminApi.updateSystemConfig(
      'system_proxy_node_id',
      systemConfig.value.system_proxy_node_id || null,
      '系统默认代理节点 ID'
    )
    if (originalConfig.value) {
      originalConfig.value.system_proxy_node_id = systemConfig.value.system_proxy_node_id
    }
    success('网络代理配置已保存')
  } catch (err) {
    error('保存代理配置失败')
    log.error('保存代理配置失败:', err)
  } finally {
    proxyConfigLoading.value = false
  }
}

async function saveBasicConfig() {
  basicConfigLoading.value = true
  try {
    const configItems = [
      {
        key: 'default_user_quota_usd',
        value: systemConfig.value.default_user_quota_usd,
        description: '默认用户配额（美元）'
      },
      {
        key: 'rate_limit_per_minute',
        value: systemConfig.value.rate_limit_per_minute,
        description: '每分钟请求限制'
      },
      {
        key: 'enable_registration',
        value: systemConfig.value.enable_registration,
        description: '是否开放用户注册'
      },
      {
        key: 'auto_delete_expired_keys',
        value: systemConfig.value.auto_delete_expired_keys,
        description: '是否自动删除过期的API Key'
      },
      {
        key: 'enable_format_conversion',
        value: systemConfig.value.enable_format_conversion,
        description: '全局格式转换开关：开启时强制允许所有提供商的格式转换'
      },
    ]

    await Promise.all(
      configItems.map(item =>
        adminApi.updateSystemConfig(item.key, item.value, item.description)
      )
    )
    // 更新原始值
    if (originalConfig.value) {
      originalConfig.value.default_user_quota_usd = systemConfig.value.default_user_quota_usd
      originalConfig.value.rate_limit_per_minute = systemConfig.value.rate_limit_per_minute
      originalConfig.value.enable_registration = systemConfig.value.enable_registration
      originalConfig.value.auto_delete_expired_keys = systemConfig.value.auto_delete_expired_keys
      originalConfig.value.enable_format_conversion = systemConfig.value.enable_format_conversion
    }
    success('基础配置已保存')
  } catch (err) {
    error('保存配置失败')
    log.error('保存基础配置失败:', err)
  } finally {
    basicConfigLoading.value = false
  }
}

async function saveLogConfig() {
  logConfigLoading.value = true
  try {
    const configItems = [
      {
        key: 'request_record_level',
        value: systemConfig.value.request_record_level,
        description: '请求记录级别'
      },
      {
        key: 'max_request_body_size',
        value: systemConfig.value.max_request_body_size,
        description: '最大请求体记录大小（字节）'
      },
      {
        key: 'max_response_body_size',
        value: systemConfig.value.max_response_body_size,
        description: '最大响应体记录大小（字节）'
      },
      {
        key: 'sensitive_headers',
        value: systemConfig.value.sensitive_headers,
        description: '敏感请求头列表'
      },
    ]

    await Promise.all(
      configItems.map(item =>
        adminApi.updateSystemConfig(item.key, item.value, item.description)
      )
    )
    // 更新原始值
    if (originalConfig.value) {
      originalConfig.value.request_record_level = systemConfig.value.request_record_level
      originalConfig.value.max_request_body_size = systemConfig.value.max_request_body_size
      originalConfig.value.max_response_body_size = systemConfig.value.max_response_body_size
      originalConfig.value.sensitive_headers = [...systemConfig.value.sensitive_headers]
    }
    success('请求记录配置已保存')
  } catch (err) {
    error('保存配置失败')
    log.error('保存请求记录配置失败:', err)
  } finally {
    logConfigLoading.value = false
  }
}

async function handleAutoCleanupToggle(enabled: boolean) {
  const previousValue = systemConfig.value.enable_auto_cleanup
  systemConfig.value.enable_auto_cleanup = enabled
  try {
    await adminApi.updateSystemConfig(
      'enable_auto_cleanup',
      enabled,
      '是否启用自动清理任务'
    )
    success(enabled ? '已启用自动清理' : '已禁用自动清理')
  } catch (err) {
    error('保存配置失败')
    log.error('保存自动清理配置失败:', err)
    // 回滚状态
    systemConfig.value.enable_auto_cleanup = previousValue
  }
}

async function handleProviderCheckinToggle(enabled: boolean) {
  const previousValue = systemConfig.value.enable_provider_checkin
  systemConfig.value.enable_provider_checkin = enabled
  try {
    await adminApi.updateSystemConfig(
      'enable_provider_checkin',
      enabled,
      '是否启用 Provider 自动签到任务'
    )
    success(enabled ? '已启用自动签到' : '已禁用自动签到')
  } catch (err) {
    error('保存配置失败')
    log.error('保存自动签到配置失败:', err)
    // 回滚状态
    systemConfig.value.enable_provider_checkin = previousValue
  }
}

const previousCheckinTime = ref('')
const checkinConfigLoading = ref(false)
const quotaResetConfigLoading = ref(false)

// 解析签到时间的小时和分钟
const checkinHour = computed(() => {
  const time = systemConfig.value.provider_checkin_time
  if (!time || !time.includes(':')) return '01'
  return time.split(':')[0]
})

const checkinMinute = computed(() => {
  const time = systemConfig.value.provider_checkin_time
  if (!time || !time.includes(':')) return '05'
  return time.split(':')[1]
})

// 更新签到时间
function updateCheckinTime(hour: string, minute: string) {
  systemConfig.value.provider_checkin_time = `${hour}:${minute}`
}

// 检测签到时间是否有变化
const hasCheckinTimeChanged = computed(() => {
  return systemConfig.value.provider_checkin_time !== previousCheckinTime.value
})

async function handleUserQuotaResetToggle(enabled: boolean) {
  const previousValue = systemConfig.value.enable_user_quota_reset
  systemConfig.value.enable_user_quota_reset = enabled
  try {
    await adminApi.updateSystemConfig(
      'enable_user_quota_reset',
      enabled,
      '是否启用用户配额自动重置任务'
    )
    success(enabled ? '已启用用户配额自动重置' : '已禁用用户配额自动重置')
  } catch (err) {
    error('保存配置失败')
    log.error('保存用户配额自动重置配置失败:', err)
    // 回滚状态
    systemConfig.value.enable_user_quota_reset = previousValue
  }
}

async function handleOAuthTokenRefreshToggle(enabled: boolean) {
  const previousValue = systemConfig.value.enable_oauth_token_refresh
  systemConfig.value.enable_oauth_token_refresh = enabled
  try {
    await adminApi.updateSystemConfig(
      'enable_oauth_token_refresh',
      enabled,
      '是否启用 OAuth Token 自动刷新任务'
    )
    success(enabled ? '已启用 OAuth Token 自动刷新' : '已禁用 OAuth Token 自动刷新')
  } catch (err) {
    error('保存配置失败')
    log.error('保存 OAuth Token 自动刷新配置失败:', err)
    // 回滚状态
    systemConfig.value.enable_oauth_token_refresh = previousValue
  }
}

// 用户配额重置时间相关
const previousUserQuotaResetTime = ref('')
const previousUserQuotaResetIntervalDays = ref(1)

const userQuotaResetHour = computed(() => {
  const time = systemConfig.value.user_quota_reset_time
  if (!time || !time.includes(':')) return '05'
  return time.split(':')[0]
})

const userQuotaResetMinute = computed(() => {
  const time = systemConfig.value.user_quota_reset_time
  if (!time || !time.includes(':')) return '00'
  return time.split(':')[1]
})

function updateUserQuotaResetTime(hour: string, minute: string) {
  systemConfig.value.user_quota_reset_time = `${hour}:${minute}`
}

const hasUserQuotaResetTimeChanged = computed(() => {
  return systemConfig.value.user_quota_reset_time !== previousUserQuotaResetTime.value
})

const hasUserQuotaResetIntervalChanged = computed(() => {
  return systemConfig.value.user_quota_reset_interval_days !== previousUserQuotaResetIntervalDays.value
})

const hasQuotaResetConfigChanged = computed(() => {
  return hasUserQuotaResetTimeChanged.value || hasUserQuotaResetIntervalChanged.value
})

// 定时任务配置列表
const scheduledTasks = computed(() => [
  {
    id: 'provider-checkin',
    icon: CalendarCheck,
    title: 'Provider 自动签到',
    description: '自动执行已配置 Provider 的签到任务',
    enabled: systemConfig.value.enable_provider_checkin,
    hasTimeConfig: true,
    hour: checkinHour.value,
    minute: checkinMinute.value,
    updateTime: updateCheckinTime,
    hasChanges: hasCheckinTimeChanged.value,
    loading: checkinConfigLoading.value,
    onToggle: handleProviderCheckinToggle,
    onSave: handleCheckinTimeSave,
  },
  {
    id: 'user-quota-reset',
    icon: RotateCcw,
    title: '用户配额自动重置',
    description: '定时将用户已使用配额重置为零',
    enabled: systemConfig.value.enable_user_quota_reset,
    hasTimeConfig: true,
    hour: userQuotaResetHour.value,
    minute: userQuotaResetMinute.value,
    updateTime: updateUserQuotaResetTime,
    hasChanges: hasQuotaResetConfigChanged.value,
    loading: quotaResetConfigLoading.value,
    onToggle: handleUserQuotaResetToggle,
    onSave: handleQuotaResetConfigSave,
  },
  {
    id: 'oauth-token-refresh',
    icon: RefreshCw,
    title: 'OAuth Token 自动刷新',
    description: '主动刷新即将过期的 OAuth Token（动态调度）',
    enabled: systemConfig.value.enable_oauth_token_refresh,
    hasTimeConfig: false,
    hour: '',
    minute: '',
    updateTime: () => {},
    hasChanges: false,
    loading: false,
    onToggle: handleOAuthTokenRefreshToggle,
    onSave: () => {},
  },
])

// Provider 签到时间保存
async function handleCheckinTimeSave() {
  const newTime = systemConfig.value.provider_checkin_time
  if (!newTime || !/^\d{2}:\d{2}$/.test(newTime)) {
    error('请输入有效的时间格式 (HH:MM)')
    return
  }

  checkinConfigLoading.value = true
  try {
    await adminApi.updateSystemConfig(
      'provider_checkin_time',
      newTime,
      'Provider 自动签到执行时间（HH:MM 格式）'
    )
    previousCheckinTime.value = newTime
    success(`签到时间已设置为 ${newTime}`)
  } catch (err) {
    error('保存签到时间失败')
    log.error('保存签到时间失败:', err)
  } finally {
    checkinConfigLoading.value = false
  }
}

// 用户配额重置配置保存
async function handleQuotaResetConfigSave() {
  const configItems: Array<{ key: string, value: any, description: string, onSuccess: () => void }> = []

  if (hasUserQuotaResetTimeChanged.value) {
    const newTime = systemConfig.value.user_quota_reset_time
    if (!newTime || !/^\d{2}:\d{2}$/.test(newTime)) {
      error('请输入有效的时间格式 (HH:MM)')
      return
    }
    configItems.push({
      key: 'user_quota_reset_time',
      value: newTime,
      description: '用户配额自动重置执行时间（HH:MM 格式）',
      onSuccess: () => {
        previousUserQuotaResetTime.value = newTime
      }
    })
  }

  if (hasUserQuotaResetIntervalChanged.value) {
    let intervalDays = Number(systemConfig.value.user_quota_reset_interval_days)
    if (!Number.isFinite(intervalDays) || intervalDays < 1) intervalDays = 1
    intervalDays = Math.trunc(intervalDays)

    systemConfig.value.user_quota_reset_interval_days = intervalDays

    configItems.push({
      key: 'user_quota_reset_interval_days',
      value: intervalDays,
      description: '用户配额重置周期（天数），滚动计算',
      onSuccess: () => {
        previousUserQuotaResetIntervalDays.value = intervalDays
      }
    })
  }

  if (configItems.length === 0) return

  quotaResetConfigLoading.value = true
  const failedKeys: string[] = []

  try {
    for (const item of configItems) {
      try {
        await adminApi.updateSystemConfig(item.key, item.value, item.description)
        item.onSuccess()
      } catch (err) {
        failedKeys.push(item.key)
        log.error(`保存配额重置配置失败: ${item.key}`, err)
      }
    }

    if (failedKeys.length > 0) {
      error(`部分配置保存失败: ${failedKeys.join(', ')}`)
      return
    }

    success('配额重置配置已保存')
  } finally {
    quotaResetConfigLoading.value = false
  }
}

async function saveCleanupConfig() {
  cleanupConfigLoading.value = true
  try {
    const configItems = [
      {
        key: 'detail_log_retention_days',
        value: systemConfig.value.detail_log_retention_days,
        description: '详细记录保留天数'
      },
      {
        key: 'compressed_log_retention_days',
        value: systemConfig.value.compressed_log_retention_days,
        description: '压缩记录保留天数'
      },
      {
        key: 'header_retention_days',
        value: systemConfig.value.header_retention_days,
        description: '请求头保留天数'
      },
      {
        key: 'log_retention_days',
        value: systemConfig.value.log_retention_days,
        description: '完整记录保留天数'
      },
      {
        key: 'cleanup_batch_size',
        value: systemConfig.value.cleanup_batch_size,
        description: '每批次清理的记录数'
      },
      {
        key: 'audit_log_retention_days',
        value: systemConfig.value.audit_log_retention_days,
        description: '审计日志保留天数'
      },
    ]

    await Promise.all(
      configItems.map(item =>
        adminApi.updateSystemConfig(item.key, item.value, item.description)
      )
    )
    // 更新原始值
    if (originalConfig.value) {
      originalConfig.value.detail_log_retention_days = systemConfig.value.detail_log_retention_days
      originalConfig.value.compressed_log_retention_days = systemConfig.value.compressed_log_retention_days
      originalConfig.value.header_retention_days = systemConfig.value.header_retention_days
      originalConfig.value.log_retention_days = systemConfig.value.log_retention_days
      originalConfig.value.cleanup_batch_size = systemConfig.value.cleanup_batch_size
      originalConfig.value.audit_log_retention_days = systemConfig.value.audit_log_retention_days
    }
    success('请求记录清理配置已保存')
  } catch (err) {
    error('保存配置失败')
    log.error('保存请求记录清理配置失败:', err)
  } finally {
    cleanupConfigLoading.value = false
  }
}

// 导出配置
async function handleExportConfig() {
  exportLoading.value = true
  try {
    const data = await adminApi.exportConfig()
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `${systemConfig.value.site_name.toLowerCase()}-config-${new Date().toISOString().slice(0, 10)}.json`
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)
    success('配置已导出')
  } catch (err) {
    error('导出配置失败')
    log.error('导出配置失败:', err)
  } finally {
    exportLoading.value = false
  }
}

// 触发文件选择
function triggerConfigFileSelect() {
  configFileInput.value?.click()
}

// 文件大小限制 (10MB)
const MAX_FILE_SIZE = 10 * 1024 * 1024

// 处理文件选择
function handleConfigFileSelect(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return

  if (file.size > MAX_FILE_SIZE) {
    error('文件大小不能超过 10MB')
    input.value = ''
    return
  }

  const reader = new FileReader()
  reader.onload = (e) => {
    try {
      const content = e.target?.result as string
      const data = JSON.parse(content) as ConfigExportData

      // 基本格式验证：确保有版本字段
      if (!data.version) {
        error('无效的配置文件：缺少版本信息')
        return
      }

      importPreview.value = data
      mergeMode.value = 'skip'
      importDialogOpen.value = true
    } catch (err) {
      error('解析配置文件失败，请确保是有效的 JSON 文件')
      log.error('解析配置文件失败:', err)
    }
  }
  reader.readAsText(file)

  // 重置 input 以便能再次选择同一文件
  input.value = ''
}

// 确认导入
async function confirmImport() {
  if (!importPreview.value) return

  importLoading.value = true
  try {
    const result = await adminApi.importConfig({
      ...importPreview.value,
      merge_mode: mergeMode.value
    })
    importResult.value = result
    importDialogOpen.value = false
    mergeModeSelectOpen.value = false
    importResultDialogOpen.value = true
    success('配置导入成功')
  } catch (err: any) {
    error(err.response?.data?.detail || '导入配置失败')
    log.error('导入配置失败:', err)
  } finally {
    importLoading.value = false
  }
}

// 导出用户数据
async function handleExportUsers() {
  exportUsersLoading.value = true
  try {
    const data = await adminApi.exportUsers()
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `${systemConfig.value.site_name.toLowerCase()}-users-${new Date().toISOString().slice(0, 10)}.json`
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)
    success('用户数据已导出')
  } catch (err) {
    error('导出用户数据失败')
    log.error('导出用户数据失败:', err)
  } finally {
    exportUsersLoading.value = false
  }
}

// 触发用户数据文件选择
function triggerUsersFileSelect() {
  usersFileInput.value?.click()
}

// 处理用户数据文件选择
function handleUsersFileSelect(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return

  if (file.size > MAX_FILE_SIZE) {
    error('文件大小不能超过 10MB')
    input.value = ''
    return
  }

  const reader = new FileReader()
  reader.onload = (e) => {
    try {
      const content = e.target?.result as string
      const data = JSON.parse(content) as UsersExportData

      importUsersPreview.value = data
      usersMergeMode.value = 'skip'
      importUsersDialogOpen.value = true
    } catch (err) {
      error('解析用户数据文件失败，请确保是有效的 JSON 文件')
      log.error('解析用户数据文件失败:', err)
    }
  }
  reader.readAsText(file)

  // 重置 input 以便能再次选择同一文件
  input.value = ''
}

// 确认导入用户数据
async function confirmImportUsers() {
  if (!importUsersPreview.value) return

  importUsersLoading.value = true
  try {
    const result = await adminApi.importUsers({
      ...importUsersPreview.value,
      merge_mode: usersMergeMode.value
    })
    importUsersResult.value = result
    importUsersDialogOpen.value = false
    usersMergeModeSelectOpen.value = false
    importUsersResultDialogOpen.value = true
    success('用户数据导入成功')
  } catch (err: any) {
    error(err.response?.data?.detail || '导入用户数据失败')
    log.error('导入用户数据失败:', err)
  } finally {
    importUsersLoading.value = false
  }
}
</script>
