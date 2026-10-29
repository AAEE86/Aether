<template>
  <Dialog
    :open="open"
    size="4xl"
    :title="plan ? '编辑套餐' : '新建套餐'"
    description="通过固定权益控件生成套餐配置"
    no-padding
    @update:open="(value: boolean) => $emit('update:open', value)"
  >
    <div class="max-h-[calc(100vh-193px)] space-y-4 overflow-y-auto px-6 py-4">
      <div class="grid grid-cols-1 gap-2 md:grid-cols-2 xl:grid-cols-4">
        <Button
          variant="outline"
          size="sm"
          class="h-12 justify-start rounded-xl px-3 text-left"
          @click="applyTemplate('daily')"
        >
          <span>
            <span class="block text-sm font-medium leading-5">每日额度月卡</span>
            <span class="block text-xs font-normal leading-4 text-muted-foreground">周期内每天重置</span>
          </span>
        </Button>
        <Button
          variant="outline"
          size="sm"
          class="h-12 justify-start rounded-xl px-3 text-left"
          @click="applyTemplate('traffic')"
        >
          <span>
            <span class="block text-sm font-medium leading-5">流量限制套餐</span>
            <span class="block text-xs font-normal leading-4 text-muted-foreground">QPS、RPM 与并发组合</span>
          </span>
        </Button>
        <Button
          variant="outline"
          size="sm"
          class="h-12 justify-start rounded-xl px-3 text-left"
          @click="applyTemplate('membership')"
        >
          <span>
            <span class="block text-sm font-medium leading-5">会员权益包</span>
            <span class="block text-xs font-normal leading-4 text-muted-foreground">动态授予分组</span>
          </span>
        </Button>
        <Button
          variant="outline"
          size="sm"
          class="h-12 justify-start rounded-xl px-3 text-left"
          @click="applyTemplate('mixed')"
        >
          <span>
            <span class="block text-sm font-medium leading-5">混合套餐</span>
            <span class="block text-xs font-normal leading-4 text-muted-foreground">周期权益组合</span>
          </span>
        </Button>
      </div>

      <div class="rounded-xl border border-border/60 bg-muted/20 px-3 py-2">
        <div class="grid grid-cols-1 gap-2 lg:grid-cols-[1fr_auto]">
          <div class="min-w-0 space-y-1.5">
            <div class="flex flex-wrap items-center gap-2">
              <div class="text-sm font-medium">
                {{ planModeGuide.title }}
              </div>
              <Badge variant="outline">
                {{ planModeGuide.badge }}
              </Badge>
            </div>
            <p class="text-xs leading-5 text-muted-foreground">
              {{ planModeGuide.description }}
            </p>
          </div>
          <div class="flex flex-wrap items-center gap-1.5 lg:justify-end">
            <span
              v-for="note in planModeGuide.notes"
              :key="note"
              class="rounded-full border border-border/60 bg-card/60 px-2.5 py-1 text-xs leading-4 text-muted-foreground"
            >
              {{ note }}
            </span>
          </div>
        </div>
      </div>

      <div
        v-if="planMode !== 'empty'"
        class="mx-auto w-full max-w-[880px] rounded-2xl border border-border/60 bg-muted/10 p-6"
      >
        <div class="grid grid-cols-1 gap-x-4 gap-y-3 xl:grid-cols-12">
          <div class="border-b border-border/70 pb-2 xl:col-span-12">
            <h3 class="text-sm font-semibold leading-5">
              基础信息
            </h3>
          </div>

          <div class="space-y-1.5 xl:col-span-8">
            <Label
              for="plan-title"
              class="inline-flex items-center gap-1.5 text-sm font-medium"
            >
              <span>套餐名称</span>
              <span class="text-destructive">*</span>
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger as-child>
                    <button
                      type="button"
                      class="text-muted-foreground/60 hover:text-muted-foreground"
                      aria-label="套餐名称说明"
                    >
                      <CircleHelp class="h-3.5 w-3.5" />
                    </button>
                  </TooltipTrigger>
                  <TooltipContent
                    side="top"
                    class="max-w-64 text-xs"
                  >
                    用户端购买页和订单快照里显示的套餐名称。
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            </Label>
            <Input
              id="plan-title"
              v-model="form.title"
              class="h-9 rounded-xl bg-muted/70"
              placeholder="Pro 月卡"
            />
          </div>

          <div class="space-y-1.5 xl:col-span-4">
            <Label
              for="plan-price"
              class="inline-flex items-center gap-1.5 text-sm font-medium"
            >
              <span>价格</span>
              <span class="text-destructive">*</span>
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger as-child>
                    <button
                      type="button"
                      class="text-muted-foreground/60 hover:text-muted-foreground"
                      aria-label="价格字段说明"
                    >
                      <CircleHelp class="h-3.5 w-3.5" />
                    </button>
                  </TooltipTrigger>
                  <TooltipContent
                    side="top"
                    class="max-w-72 text-xs"
                  >
                    设置用户实际支付的套餐价格。
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            </Label>
            <div class="grid grid-cols-[minmax(0,1fr)_88px]">
              <Input
                id="plan-price"
                v-model.number="form.price_amount"
                class="h-9 rounded-l-xl rounded-r-none border-r-0 bg-muted/70 focus-visible:z-10"
                type="number"
                inputmode="decimal"
                min="0.01"
                step="0.01"
                @blur="normalizePriceAmount"
              />
              <Select v-model="form.price_currency">
                <SelectTrigger class="h-9 rounded-l-none rounded-r-xl border-l-0 bg-muted/70 px-3 focus:z-10">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem
                    v-for="currency in priceCurrencyOptions"
                    :key="currency"
                    :value="currency"
                  >
                    {{ currency }}
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>

          <div class="space-y-1.5 xl:col-span-12">
            <Label
              for="plan-description"
              class="inline-flex items-center gap-1.5 text-sm font-medium"
            >
              <span>说明</span>
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger as-child>
                    <button
                      type="button"
                      class="text-muted-foreground/60 hover:text-muted-foreground"
                      aria-label="套餐说明字段说明"
                    >
                      <CircleHelp class="h-3.5 w-3.5" />
                    </button>
                  </TooltipTrigger>
                  <TooltipContent
                    side="top"
                    class="max-w-72 text-xs"
                  >
                    简短描述套餐包含的权益，建议控制在一两句话内。
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            </Label>
            <Textarea
              id="plan-description"
              v-model="form.description"
              class="min-h-[72px] resize-y rounded-2xl bg-muted/70"
              rows="2"
              placeholder="简短说明套餐权益"
            />
          </div>
        </div>

        <section class="mt-5 space-y-3">
          <div class="border-b border-border/70 pb-2">
            <h3 class="text-sm font-semibold leading-5">
              购买限制
            </h3>
          </div>
          <div class="grid grid-cols-1 gap-x-4 gap-y-3 xl:grid-cols-12">
            <div
              class="space-y-1.5"
              :class="purchaseLimitFieldSpanClass"
            >
              <Label
                for="plan-purchase-limit-scope"
                class="inline-flex items-center gap-1.5 text-sm font-medium"
              >
                <span>重复购买限制</span>
                <TooltipProvider>
                  <Tooltip>
                    <TooltipTrigger as-child>
                      <button
                        type="button"
                        class="text-muted-foreground/60 hover:text-muted-foreground"
                        aria-label="重复购买限制说明"
                      >
                        <CircleHelp class="h-3.5 w-3.5" />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent
                      side="top"
                      class="max-w-72 text-xs"
                    >
                      控制同一用户能否重复购买本套餐；不决定余额、每日额度或会员分组怎么发放。
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              </Label>
              <Select v-model="form.purchase_limit_scope">
                <SelectTrigger
                  id="plan-purchase-limit-scope"
                  class="h-9 rounded-xl bg-muted/70 px-3"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="active_period">
                    按周期限制
                  </SelectItem>
                  <SelectItem value="lifetime">
                    永久限制
                  </SelectItem>
                  <SelectItem value="unlimited">
                    不限购
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div
              class="space-y-1.5"
              :class="purchaseLimitFieldSpanClass"
            >
              <Label
                for="plan-duration"
                class="inline-flex items-center gap-1.5 text-sm font-medium"
              >
                <span>{{ durationFieldLabel }}</span>
                <span class="text-destructive">*</span>
                <TooltipProvider>
                  <Tooltip>
                    <TooltipTrigger as-child>
                      <button
                        type="button"
                        class="text-muted-foreground/60 hover:text-muted-foreground"
                        aria-label="周期窗口说明"
                      >
                        <CircleHelp class="h-3.5 w-3.5" />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent
                      side="top"
                      class="max-w-72 text-xs"
                    >
                      {{ durationTooltipText }}
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              </Label>
              <div class="grid grid-cols-[minmax(0,1fr)_88px]">
                <Input
                  id="plan-duration"
                  v-model.number="form.duration_value"
                  class="h-9 rounded-l-xl rounded-r-none border-r-0 bg-muted/70 focus-visible:z-10"
                  type="number"
                  inputmode="numeric"
                  min="1"
                  step="1"
                  @blur="normalizeDurationValue"
                />
                <Select v-model="form.duration_unit">
                  <SelectTrigger class="h-9 rounded-l-none rounded-r-xl border-l-0 bg-muted/70 px-3 focus:z-10">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="day">
                      日
                    </SelectItem>
                    <SelectItem value="month">
                      月
                    </SelectItem>
                    <SelectItem value="year">
                      年
                    </SelectItem>
                    <SelectItem value="custom">
                      自定义天数
                    </SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>

            <div
              v-if="showPurchaseLimitCount"
              class="space-y-1.5"
              :class="purchaseLimitFieldSpanClass"
            >
              <Label
                for="plan-max-active"
                class="inline-flex items-center gap-1.5 text-sm font-medium"
              >
                <span>{{ activeLimitFieldLabel }}</span>
                <TooltipProvider>
                  <Tooltip>
                    <TooltipTrigger as-child>
                      <button
                        type="button"
                        class="text-muted-foreground/60 hover:text-muted-foreground"
                        aria-label="最多持有份数字段说明"
                      >
                        <CircleHelp class="h-3.5 w-3.5" />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent
                      side="top"
                      class="max-w-72 text-xs"
                    >
                      {{ activeLimitTooltipText }}
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              </Label>
              <Input
                id="plan-max-active"
                v-model.number="form.max_active_per_user"
                class="h-9 rounded-xl bg-muted/70"
                type="number"
                inputmode="numeric"
                min="1"
                step="1"
                @blur="normalizeActiveLimit"
              />
            </div>
            <div class="xl:col-span-12 rounded-xl border border-border/60 bg-muted/20 px-3 py-2 text-xs leading-5 text-muted-foreground">
              <span class="font-medium text-foreground/80">当前逻辑：</span>
              {{ purchaseLimitSummaryText }}
            </div>
            <div class="xl:col-span-12 rounded-xl border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs leading-5 text-amber-200">
              每日额度和会员权益仍按类型互斥；使用限制套餐默认可同时生效，相同指标与窗口按最严格上限执行。仅同名套餐互斥组会替换旧套餐；替换时组合权益整体失效。
            </div>
          </div>
        </section>

        <section class="mt-5 space-y-3">
          <div class="border-b border-border/70 pb-2">
            <h3 class="text-sm font-semibold leading-5">
              展示与上架
            </h3>
          </div>
          <div class="grid grid-cols-1 gap-x-4 gap-y-3 xl:grid-cols-12">
            <div class="space-y-1.5 xl:col-span-6">
              <Label
                for="plan-sort"
                class="inline-flex items-center gap-1.5 text-sm font-medium"
              >
                <span>展示排序</span>
                <TooltipProvider>
                  <Tooltip>
                    <TooltipTrigger as-child>
                      <button
                        type="button"
                        class="text-muted-foreground/60 hover:text-muted-foreground"
                        aria-label="展示排序说明"
                      >
                        <CircleHelp class="h-3.5 w-3.5" />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent
                      side="top"
                      class="max-w-64 text-xs"
                    >
                      数值越小越靠前，用户端套餐列表按排序值升序展示。
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              </Label>
              <Input
                id="plan-sort"
                v-model.number="form.sort_order"
                class="h-9 rounded-xl bg-muted/70"
                type="number"
                step="1"
              />
            </div>
            <div class="space-y-1.5 xl:col-span-6">
              <Label class="inline-flex items-center gap-1.5 text-sm font-medium">
                <span>上架状态</span>
                <TooltipProvider>
                  <Tooltip>
                    <TooltipTrigger as-child>
                      <button
                        type="button"
                        class="text-muted-foreground/60 hover:text-muted-foreground"
                        aria-label="上架状态说明"
                      >
                        <CircleHelp class="h-3.5 w-3.5" />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent
                      side="top"
                      class="max-w-64 text-xs"
                    >
                      停用后保留配置，但用户端套餐中心不再展示。
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              </Label>
              <div class="flex h-9 items-center justify-between rounded-xl border border-border/60 bg-muted/70 px-3">
                <span class="text-sm text-muted-foreground">
                  {{ form.enabled ? '已上架' : '未上架' }}
                </span>
                <Switch v-model="form.enabled" />
              </div>
            </div>
          </div>
        </section>
      </div>

      <section
        v-if="planMode !== 'empty'"
        class="space-y-4"
      >
        <h3 class="text-sm font-semibold">
          权益配置
        </h3>

        <div
          v-if="showWalletCreditConfig"
          class="space-y-3 rounded-2xl border border-border/60 bg-muted/20 p-4"
        >
          <div class="flex items-center justify-between gap-3">
            <div>
              <Label class="text-sm font-medium">附赠余额</Label>
              <p class="mt-1 text-xs text-muted-foreground">
                {{ walletCreditSummaryText }}
              </p>
            </div>
            <Switch v-model="form.wallet_credit_enabled" />
          </div>
          <div
            v-if="form.wallet_credit_enabled"
            class="grid grid-cols-1 gap-3 md:grid-cols-2"
          >
            <div class="space-y-1.5">
              <Label>发放金额 (USD)</Label>
              <Input
                v-model.number="form.wallet_credit_amount_usd"
                type="number"
                min="0.01"
                step="0.01"
              />
            </div>
            <div class="space-y-1.5">
              <Label>余额类型</Label>
              <Select v-model="form.wallet_credit_balance_bucket">
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="recharge">
                    充值余额
                  </SelectItem>
                  <SelectItem value="gift">
                    赠款余额
                  </SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div class="space-y-1.5 md:col-span-2">
              <Label>套餐互斥组（可选）</Label>
              <Input
                v-model="form.wallet_credit_replacement_group"
                maxlength="128"
                placeholder="pro-tier"
              />
              <p class="text-xs leading-5 text-muted-foreground">
                与新套餐的任一权益使用同名组时，旧套餐及其组合权益会整体失效。
              </p>
            </div>
            <p class="rounded-xl border border-border/50 bg-card/60 px-3 py-2 text-xs leading-5 text-muted-foreground md:col-span-2">
              {{ walletCreditDetailText }}
            </p>
          </div>
        </div>

        <div
          v-if="showDailyQuotaConfig"
          class="space-y-3 rounded-2xl border border-border/60 bg-muted/20 p-4"
        >
          <div class="flex items-center justify-between gap-3">
            <div>
              <Label class="text-sm font-medium">每日额度</Label>
              <p class="mt-1 text-xs text-muted-foreground">
                {{ dailyQuotaSummaryText }}
              </p>
            </div>
            <Switch v-model="form.daily_quota_enabled" />
          </div>
          <div
            v-if="form.daily_quota_enabled"
            class="grid grid-cols-1 gap-3 md:grid-cols-2"
          >
            <div class="space-y-1.5">
              <Label>每日额度 (USD)</Label>
              <Input
                v-model.number="form.daily_quota_usd"
                type="number"
                min="0.01"
                step="0.01"
              />
            </div>
            <div class="space-y-1.5">
              <Label>重置时区</Label>
              <Input
                v-model="form.reset_timezone"
                placeholder="Asia/Shanghai"
              />
            </div>
            <div class="space-y-1.5 md:col-span-2">
              <Label>套餐互斥组（可选）</Label>
              <Input
                v-model="form.daily_quota_replacement_group"
                maxlength="128"
                placeholder="pro-tier"
              />
              <p class="text-xs leading-5 text-muted-foreground">
                每日额度套餐本身已按类型互斥；填写后还可与其他权益类型的同名组整包互斥。
              </p>
            </div>
            <p class="rounded-xl border border-border/60 bg-card/50 p-3 text-xs leading-5 text-muted-foreground">
              套餐额度不足时是否使用钱包余额，由用户在个人设置中选择，默认关闭。
            </p>
            <div class="flex items-center justify-between rounded-xl border border-border/60 bg-card/50 p-3 opacity-70">
              <div>
                <Label>额度结转</Label>
                <p class="mt-1 text-xs text-muted-foreground">
                  当前后端固定不支持结转
                </p>
              </div>
              <Switch
                v-model="form.carry_over"
                disabled
              />
            </div>
            <p class="rounded-xl border border-border/50 bg-card/60 px-3 py-2 text-xs leading-5 text-muted-foreground md:col-span-2">
              {{ dailyQuotaDetailText }}
            </p>
          </div>
        </div>

        <div
          v-if="showMembershipGroupConfig"
          class="space-y-3 rounded-2xl border border-border/60 bg-muted/20 p-4"
        >
          <div class="flex items-center justify-between gap-3">
            <div>
              <Label class="text-sm font-medium">会员分组</Label>
              <p class="mt-1 text-xs text-muted-foreground">
                {{ membershipSummaryText }}
              </p>
            </div>
            <Switch v-model="form.membership_group_enabled" />
          </div>
          <div
            v-if="form.membership_group_enabled"
            class="space-y-3"
          >
            <p class="rounded-xl border border-border/50 bg-card/60 px-3 py-2 text-xs leading-5 text-muted-foreground">
              {{ membershipDetailText }}
            </p>
            <MultiSelect
              v-model="form.grant_user_groups"
              :options="userGroupOptions"
              placeholder="选择要授予的用户分组"
              empty-text="暂无用户分组"
            />
            <div class="grid grid-cols-1 gap-2 md:grid-cols-[1fr_auto]">
              <Input
                v-model="manualGroupId"
                placeholder="手动输入分组 ID"
                @keyup.enter="addManualGroup"
              />
              <Button
                variant="outline"
                @click="addManualGroup"
              >
                添加
              </Button>
            </div>
            <div class="space-y-1.5">
              <Label>套餐互斥组（可选）</Label>
              <Input
                v-model="form.membership_group_replacement_group"
                maxlength="128"
                placeholder="pro-tier"
              />
              <p class="text-xs leading-5 text-muted-foreground">
                会员权益包本身已按类型互斥；填写后还可与其他权益类型的同名组整包互斥。
              </p>
            </div>
          </div>
        </div>

        <div class="space-y-3 rounded-2xl border border-border/60 bg-muted/20 p-4">
          <div class="flex items-center justify-between gap-3">
            <div>
              <Label class="text-sm font-medium">使用限制</Label>
              <p class="mt-1 text-xs text-muted-foreground">
                消费额度、QPS、RPM、周期请求数和并发硬限制
              </p>
            </div>
            <Switch v-model="form.usage_policy_enabled" />
          </div>

          <div
            v-if="form.usage_policy_enabled"
            class="space-y-3"
          >
            <div
              v-for="(policy, policyIndex) in form.usage_policies"
              :key="policy.local_id"
              class="space-y-3 rounded-xl border border-border/60 bg-card/50 p-3"
            >
              <div class="flex items-center justify-between gap-3">
                <div class="text-sm font-medium">
                  策略 {{ policyIndex + 1 }}
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  class="h-8 w-8 p-0"
                  :aria-label="`删除策略 ${policyIndex + 1}`"
                  :title="`删除策略 ${policyIndex + 1}`"
                  @click="removeUsagePolicy(policyIndex)"
                >
                  <Trash2 class="h-4 w-4" />
                </Button>
              </div>

              <div class="grid grid-cols-1 gap-3 md:grid-cols-3">
                <div class="space-y-1.5">
                  <Label>策略名称</Label>
                  <Input
                    v-model="policy.name"
                    maxlength="128"
                    placeholder="标准流量限制"
                  />
                </div>
                <div class="space-y-1.5">
                  <Label>策略标识</Label>
                  <Input
                    v-model="policy.policy_id"
                    maxlength="128"
                    placeholder="standard-traffic"
                  />
                </div>
                <div class="space-y-1.5">
                  <Label>套餐互斥组（可选）</Label>
                  <Input
                    v-model="policy.replacement_group"
                    maxlength="128"
                    placeholder="pro-tier"
                  />
                  <p class="text-xs leading-5 text-muted-foreground">
                    同组套餐换购时旧套餐整包失效；留空时此策略可与其他策略同时生效，相同指标与窗口按较低上限执行。每日额度和会员权益仍按类型互斥。
                  </p>
                </div>
              </div>

              <div class="space-y-3">
                <div class="space-y-3 lg:space-y-2">
                  <div
                    v-for="(rule, ruleIndex) in policy.rules"
                    :key="rule.local_id"
                    class="grid grid-cols-1 gap-3 rounded-xl border border-border/60 bg-background/50 p-3 lg:grid-cols-[150px_190px_minmax(260px,1fr)_110px_36px] lg:items-end lg:gap-2 lg:rounded-none lg:border-0 lg:bg-transparent lg:p-0"
                  >
                    <div class="space-y-1.5">
                      <Label>指标</Label>
                      <Select
                        v-model="rule.metric"
                        @update:model-value="onUsageMetricChanged(rule)"
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="request_count">
                            请求数
                          </SelectItem>
                          <SelectItem value="actual_cost_usd">
                            消费金额 (USD)
                          </SelectItem>
                          <SelectItem value="concurrency">
                            并发数
                          </SelectItem>
                        </SelectContent>
                      </Select>
                    </div>

                    <div class="space-y-1.5">
                      <Label>窗口</Label>
                      <Select
                        v-model="rule.window_kind"
                        @update:model-value="onUsageWindowChanged(rule)"
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem
                            v-if="rule.metric !== 'concurrency'"
                            value="rolling"
                          >
                            滚动窗口
                          </SelectItem>
                          <SelectItem
                            v-if="rule.metric !== 'concurrency'"
                            value="calendar_day"
                          >
                            自然日
                          </SelectItem>
                          <SelectItem
                            v-if="rule.metric !== 'concurrency'"
                            value="calendar_week"
                          >
                            自然周
                          </SelectItem>
                          <SelectItem
                            v-if="rule.metric !== 'concurrency'"
                            value="calendar_month"
                          >
                            自然月
                          </SelectItem>
                          <SelectItem
                            v-if="rule.metric !== 'concurrency'"
                            value="subscription_period"
                          >
                            套餐周期
                          </SelectItem>
                          <SelectItem
                            v-if="rule.metric === 'concurrency'"
                            value="concurrent"
                          >
                            同时进行
                          </SelectItem>
                        </SelectContent>
                      </Select>
                    </div>

                    <div class="space-y-1.5">
                      <Label>{{ usageWindowParameterLabel(rule) }}</Label>
                      <Input
                        v-if="rule.window_kind === 'rolling'"
                        v-model.number="rule.rolling_seconds"
                        type="number"
                        inputmode="numeric"
                        min="1"
                        max="2592000"
                        step="1"
                      />
                      <div
                        v-else-if="rule.window_kind === 'calendar_week'"
                        class="grid grid-cols-[minmax(140px,1fr)_108px] gap-2"
                      >
                        <Input
                          v-model="rule.timezone"
                          placeholder="继承系统时区"
                        />
                        <Select
                          :model-value="String(rule.week_start)"
                          @update:model-value="rule.week_start = Number($event)"
                        >
                          <SelectTrigger>
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem
                              v-for="option in weekStartOptions"
                              :key="option.value"
                              :value="option.value"
                            >
                              {{ option.label }}
                            </SelectItem>
                          </SelectContent>
                        </Select>
                      </div>
                      <Input
                        v-else-if="rule.window_kind === 'calendar_day' || rule.window_kind === 'calendar_month'"
                        v-model="rule.timezone"
                        placeholder="继承系统时区"
                      />
                      <Input
                        v-else
                        model-value="-"
                        disabled
                      />
                    </div>

                    <div class="space-y-1.5">
                      <Label>{{ rule.metric === 'actual_cost_usd' ? '上限 (USD)' : '上限' }}</Label>
                      <Input
                        v-model.number="rule.limit"
                        type="number"
                        inputmode="numeric"
                        :min="rule.metric === 'actual_cost_usd' ? 0.00000001 : 1"
                        :step="rule.metric === 'actual_cost_usd' ? 0.01 : 1"
                      />
                    </div>

                    <Button
                      variant="ghost"
                      size="sm"
                      class="h-9 w-9 justify-self-end p-0"
                      :aria-label="`删除规则 ${ruleIndex + 1}`"
                      :title="`删除规则 ${ruleIndex + 1}`"
                      @click="removeUsagePolicyRule(policy, ruleIndex)"
                    >
                      <Trash2 class="h-4 w-4" />
                    </Button>
                  </div>
                </div>
              </div>

              <div class="flex flex-wrap items-center gap-2">
                <DropdownMenu>
                  <DropdownMenuTrigger as-child>
                    <Button
                      variant="outline"
                      size="sm"
                      :disabled="usagePolicyRuleLimitReached(policy)"
                    >
                      <Plus class="mr-2 h-4 w-4" />
                      添加规则
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="start">
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'qps')">
                      QPS
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'rpm')">
                      RPM
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'rolling_5h')">
                      滚动 5 小时请求数
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'cost_5h')">
                      5 小时消费额度
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'cost_daily')">
                      每日消费额度
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'cost_weekly')">
                      每周消费额度
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'cost_monthly')">
                      每月消费额度
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'cost_subscription')">
                      套餐周期消费额度
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'daily')">
                      每日请求数
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'weekly')">
                      每周请求数
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'monthly')">
                      每月请求数
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'subscription')">
                      套餐周期请求数
                    </DropdownMenuItem>
                    <DropdownMenuItem @select="addUsagePolicyRule(policy, 'concurrency')">
                      并发数
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
                <span
                  v-if="policy.rules.length >= MAX_USAGE_POLICY_RULES_PER_POLICY"
                  class="text-xs text-muted-foreground"
                >每个策略最多 {{ MAX_USAGE_POLICY_RULES_PER_POLICY }} 条规则</span>
                <span
                  v-else-if="totalUsagePolicyRules >= MAX_USAGE_POLICY_TOTAL_RULES"
                  class="text-xs text-muted-foreground"
                >每个套餐的规则合计最多 {{ MAX_USAGE_POLICY_TOTAL_RULES }} 条</span>
              </div>
            </div>

            <Button
              variant="outline"
              size="sm"
              :disabled="usagePolicyLimitReached"
              @click="addUsagePolicy"
            >
              <Plus class="mr-2 h-4 w-4" />
              添加策略
            </Button>
            <span
              v-if="form.usage_policies.length >= MAX_USAGE_POLICY_ENTITLEMENTS"
              class="text-xs text-muted-foreground"
            >每个套餐最多 {{ MAX_USAGE_POLICY_ENTITLEMENTS }} 份策略</span>
            <span
              v-else-if="totalUsagePolicyRules >= MAX_USAGE_POLICY_TOTAL_RULES"
              class="text-xs text-muted-foreground"
            >每个套餐的规则合计最多 {{ MAX_USAGE_POLICY_TOTAL_RULES }} 条</span>
          </div>
        </div>
      </section>
    </div>

    <div class="flex h-14 items-center justify-end gap-3 border-t border-border bg-muted/10 px-6">
      <Button
        variant="outline"
        class="h-9"
        :disabled="saving"
        @click="$emit('update:open', false)"
      >
        取消
      </Button>
      <Button
        variant="default"
        class="h-9"
        :disabled="saving || isSaveDisabled"
        @click="savePlan"
      >
        {{ saving ? '保存中...' : '保存套餐' }}
      </Button>
    </div>
  </Dialog>
</template>

<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { CircleHelp, Plus, Trash2 } from 'lucide-vue-next'
import {
  adminBillingPlansApi,
  type BillingEntitlement,
  type BillingPlan,
  type BillingPlanWriteRequest,
  type DailyQuotaEntitlement,
  type MembershipGroupEntitlement,
  type WalletCreditEntitlement,
} from '@/api/billing'
import type { UserGroup } from '@/api/users'
import {
  Badge,
  Button,
  Dialog,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
  Textarea,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui'
import { MultiSelect } from '@/components/common'
import { useToast } from '@/composables/useToast'
import { parseApiError } from '@/utils/errorParser'
import { log } from '@/utils/logger'
import { formatBillingPlanDuration } from '@/utils/billingEntitlements'
import {
  MAX_USAGE_POLICY_COST_USD,
  MAX_USAGE_POLICY_ENTITLEMENTS,
  MAX_USAGE_POLICY_EXACT_INTEGER,
  MAX_USAGE_POLICY_RULES_PER_POLICY,
  MAX_USAGE_POLICY_TOTAL_RULES,
  MIN_USAGE_POLICY_COST_USD,
  attachReplacementGroup,
  buildDefaultForm,
  buildDefaultUsagePolicy,
  buildDefaultUsageRule,
  buildTemplateForm,
  buildUsagePolicyEntitlement,
  formFromPlan,
  getPlanModeGuide,
  hasPackageEntitlement,
  isValidIanaTimezone,
  usageWindowParameterLabel,
  weekStartOptions,
} from '../utils/billingPlanForm'
import type {
  PlanFormState,
  PlanMode,
  PlanModeGuide,
  TemplateKey,
  UsagePolicyForm,
  UsagePolicyRuleForm,
  UsagePolicyRulePreset,
} from '../utils/billingPlanForm'

/** 套餐创建 / 编辑对话框：模板、价格与购买限制、各类权益配置，保存后通知父组件刷新列表 */
const props = defineProps<{
  open: boolean
  plan: BillingPlan | null
  userGroups: UserGroup[]
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
  saved: []
}>()

const { success, error: showError } = useToast()

const saving = ref(false)

const manualGroupId = ref('')

const form = reactive<PlanFormState>(buildDefaultForm())
const planModeGuide = computed<PlanModeGuide>(() => getPlanModeGuide(planMode.value))

watch(() => [props.open, props.plan] as const, ([open, plan]) => {
  if (!open) return
  manualGroupId.value = ''
  assignForm(plan ? formFromPlan(plan) : buildDefaultForm())
}, { immediate: true })

function applyTemplate(template: TemplateKey) {
  assignForm(buildTemplateForm(template))
}

const userGroupOptions = computed(() =>
  props.userGroups.map((group) => ({
    value: group.id,
    label: group.name,
  }))
)

const priceCurrencyOptions = computed(() => {
  const normalized = form.price_currency.trim().toUpperCase()
  const options = ['CNY', 'USD']
  return normalized && !options.includes(normalized) ? [...options, normalized] : options
})

const hasValidPriceAmount = computed(() => {
  const value = Number(form.price_amount)
  if (!Number.isFinite(value) || value <= 0) return false
  return /^\d+(\.\d{1,2})?$/.test(String(form.price_amount))
})

const hasValidDuration = computed(() =>
  Number.isInteger(Number(form.duration_value)) && Number(form.duration_value) > 0
)

const hasValidDurationUnit = computed(() =>
  ['day', 'month', 'year', 'custom'].includes(form.duration_unit)
)

const hasValidActiveLimit = computed(() =>
  Number.isInteger(Number(form.max_active_per_user)) && Number(form.max_active_per_user) > 0
)

const hasValidPurchaseLimitScope = computed(() =>
  ['active_period', 'lifetime', 'unlimited'].includes(form.purchase_limit_scope)
)

const hasSelectedPackageEntitlement = computed(() =>
  form.daily_quota_enabled || form.membership_group_enabled || form.usage_policy_enabled
)

const showPurchaseLimitCount = computed(() =>
  form.purchase_limit_scope !== 'unlimited'
)

const purchaseLimitFieldSpanClass = computed(() =>
  showPurchaseLimitCount.value ? 'xl:col-span-4' : 'xl:col-span-6'
)

const isSaveDisabled = computed(() =>
  !form.title.trim()
  || !form.price_currency.trim()
  || !hasValidPriceAmount.value
  || !hasValidDuration.value
  || !hasValidDurationUnit.value
  || (showPurchaseLimitCount.value && !hasValidActiveLimit.value)
  || !hasValidPurchaseLimitScope.value
  || !hasSelectedPackageEntitlement.value
)

const totalUsagePolicyRules = computed(() =>
  form.usage_policies.reduce((total, policy) => total + policy.rules.length, 0)
)

const usagePolicyLimitReached = computed(() =>
  form.usage_policies.length >= MAX_USAGE_POLICY_ENTITLEMENTS
  || totalUsagePolicyRules.value >= MAX_USAGE_POLICY_TOTAL_RULES
)

const planMode = computed<PlanMode>(() => {
  const enabledCount = [
    form.wallet_credit_enabled,
    form.daily_quota_enabled,
    form.membership_group_enabled,
    form.usage_policy_enabled,
  ].filter(Boolean).length

  if (enabledCount === 0) return 'empty'
  if (enabledCount > 1) return 'mixed'
  if (form.wallet_credit_enabled) return 'wallet'
  if (form.daily_quota_enabled) return 'daily'
  if (form.membership_group_enabled) return 'membership'
  return 'usage'
})

const durationFieldLabel = computed(() => '权益有效期')

const durationTooltipText = computed(() => {
  if (planMode.value === 'wallet') {
    return '旧余额套餐仍会保存这个有效期，但余额发放后不会在到期时扣回；建议改用钱包充值功能。'
  }
  return '购买后套餐权益生效这么久，与重复购买限制的统计方式相互独立。自定义单位按天计算。'
})

const activeLimitFieldLabel = computed(() =>
  form.purchase_limit_scope === 'lifetime' ? '每人最多购买次数' : '每人最多同时生效'
)

const activeLimitTooltipText = computed(() =>
  form.purchase_limit_scope === 'lifetime'
    ? '按同一用户历史成功购买次数累计，达到该值后不能再次购买，适合首购特惠包。'
    : '只统计仍在周期内的已生效权益，过期后释放名额，用于防止周期权益无限叠加。'
)

const purchaseLimitSummaryText = computed(() => {
  if (form.purchase_limit_scope === 'unlimited') {
    return '不检查同一用户的重复购买次数；每次支付成功都会按下方权益配置发放。'
  }
  if (form.purchase_limit_scope === 'lifetime') {
    return `同一用户历史成功购买本套餐达到 ${form.max_active_per_user || 1} 次后不能再买；适合首购特惠或一次性礼包。`
  }
  return `同一用户最多同时生效 ${form.max_active_per_user || 1} 份本套餐；每份权益在 ${formatBillingPlanDuration(form.duration_unit, form.duration_value || 1)} 后到期并释放名额。`
})

const showWalletCreditConfig = computed(() =>
  planMode.value === 'mixed' || form.wallet_credit_enabled
)

const showDailyQuotaConfig = computed(() =>
  planMode.value === 'mixed' || form.daily_quota_enabled
)

const showMembershipGroupConfig = computed(() =>
  planMode.value === 'mixed' || form.membership_group_enabled
)

const walletCreditSummaryText = computed(() =>
  planMode.value === 'mixed'
    ? '作为套餐附赠余额一次性发放'
    : '旧余额套餐会一次性发放到充值余额或赠款余额'
)

const walletCreditDetailText = computed(() => {
  const bucket = form.wallet_credit_balance_bucket === 'recharge' ? '充值余额' : '赠款余额'
  return `当前会发放到${bucket}。附赠余额发放后进入钱包，不会因为套餐周期结束而自动扣回。`
})

const dailyQuotaSummaryText = computed(() =>
  planMode.value === 'mixed'
    ? '组合套餐内的周期性每日 USD 消费用量'
    : '每天独立 USD 消费用量，默认不结转'
)

const dailyQuotaDetailText = '优先使用每日套餐额度；额度不足时是否使用钱包余额，由用户在个人设置中选择，默认关闭。'

const membershipSummaryText = computed(() =>
  planMode.value === 'mixed'
    ? '组合套餐内的动态会员权限'
    : '购买后动态合并分组权限，到期自动失效'
)

const membershipDetailText = computed(() =>
  '这里授予的是动态分组权限，不会永久改写用户基础分组；权益到期后权限解析会自动移除。'
)

function assignForm(next: PlanFormState) {
  Object.assign(form, next)
}

function buildEntitlements(): BillingEntitlement[] {
  const entitlements: BillingEntitlement[] = []
  if (form.wallet_credit_enabled) {
    const entitlement: WalletCreditEntitlement = {
      type: 'wallet_credit',
      amount_usd: Number(form.wallet_credit_amount_usd),
      balance_bucket: form.wallet_credit_balance_bucket,
    }
    attachReplacementGroup(entitlement, form.wallet_credit_replacement_group)
    entitlements.push(entitlement)
  }
  if (form.daily_quota_enabled) {
    const entitlement: DailyQuotaEntitlement = {
      type: 'daily_quota',
      daily_quota_usd: Number(form.daily_quota_usd),
      reset_timezone: form.reset_timezone.trim() || 'Asia/Shanghai',
      carry_over: false,
    }
    attachReplacementGroup(entitlement, form.daily_quota_replacement_group)
    entitlements.push(entitlement)
  }
  if (form.membership_group_enabled) {
    const entitlement: MembershipGroupEntitlement = {
      type: 'membership_group',
      grant_user_groups: form.grant_user_groups.map((value) => value.trim()).filter(Boolean),
    }
    attachReplacementGroup(entitlement, form.membership_group_replacement_group)
    entitlements.push(entitlement)
  }
  if (form.usage_policy_enabled) {
    for (const policy of form.usage_policies) {
      entitlements.push(buildUsagePolicyEntitlement(policy))
    }
  }
  return entitlements
}

function addUsagePolicy() {
  if (usagePolicyLimitReached.value) return
  form.usage_policies = [...form.usage_policies, buildDefaultUsagePolicy()]
}

function removeUsagePolicy(index: number) {
  form.usage_policies = form.usage_policies.filter((_, itemIndex) => itemIndex !== index)
}

function addUsagePolicyRule(policy: UsagePolicyForm, preset: UsagePolicyRulePreset) {
  if (usagePolicyRuleLimitReached(policy)) return
  policy.rules = [...policy.rules, buildDefaultUsageRule(preset)]
}

function usagePolicyRuleLimitReached(policy: UsagePolicyForm): boolean {
  return policy.rules.length >= MAX_USAGE_POLICY_RULES_PER_POLICY
    || totalUsagePolicyRules.value >= MAX_USAGE_POLICY_TOTAL_RULES
}

function removeUsagePolicyRule(policy: UsagePolicyForm, index: number) {
  policy.rules = policy.rules.filter((_, itemIndex) => itemIndex !== index)
}

function onUsageMetricChanged(rule: UsagePolicyRuleForm) {
  if (rule.metric === 'concurrency') {
    rule.window_kind = 'concurrent'
  } else if (rule.window_kind === 'concurrent') {
    rule.window_kind = 'rolling'
  }
  if (rule.metric === 'actual_cost_usd' && Number.isInteger(rule.limit)) {
    rule.limit = 10
  }
}

function onUsageWindowChanged(rule: UsagePolicyRuleForm) {
  if (rule.window_kind === 'concurrent') {
    rule.metric = 'concurrency'
  } else if (rule.metric === 'concurrency') {
    rule.metric = 'request_count'
  }
}

function normalizePriceAmount() {
  const value = Number(form.price_amount)
  if (!Number.isFinite(value) || value <= 0) return
  form.price_amount = Number(value.toFixed(2))
}

function normalizeDurationValue() {
  const value = Number(form.duration_value)
  if (!Number.isFinite(value) || value <= 0) return
  form.duration_value = Math.floor(value)
}

function normalizeActiveLimit() {
  const value = Number(form.max_active_per_user)
  if (!Number.isFinite(value) || value <= 0) return
  form.max_active_per_user = Math.floor(value)
}

function validatePlan(entitlements: BillingEntitlement[]): string | null {
  if (!form.title.trim()) return '请输入套餐名称'
  if (!Number.isFinite(Number(form.price_amount)) || Number(form.price_amount) <= 0) return '价格必须大于 0'
  if (!hasValidPriceAmount.value) return '价格最多支持两位小数'
  if (!form.price_currency.trim()) return '请输入价格币种'
  if (!hasValidPurchaseLimitScope.value) return '重复购买限制必须是按周期限制、永久限制或不限购'
  if (!hasValidDurationUnit.value) return '权益有效期单位必须是日/月/年/自定义天数'
  if (!hasValidDuration.value) return '权益有效期必须是正整数'
  if (showPurchaseLimitCount.value && !hasValidActiveLimit.value) {
    return `${activeLimitFieldLabel.value}必须是正整数`
  }
  if (entitlements.length === 0) return '至少启用一种权益'
  if (!hasPackageEntitlement(entitlements)) return '套餐至少需要包含每日额度、会员分组或使用限制；钱包充值请使用充值功能'
  if (form.wallet_credit_enabled && Number(form.wallet_credit_amount_usd) <= 0) return '附赠余额金额必须大于 0'
  if (form.daily_quota_enabled && Number(form.daily_quota_usd) <= 0) return '每日额度必须大于 0'
  if (form.membership_group_enabled && form.grant_user_groups.length === 0) return '会员分组权益至少选择一个分组'
  if (
    form.wallet_credit_enabled
    && form.wallet_credit_replacement_group.trim().length > 128
  ) return '附赠余额的套餐互斥组不能超过 128 个字符'
  if (
    form.daily_quota_enabled
    && form.daily_quota_replacement_group.trim().length > 128
  ) return '每日额度的套餐互斥组不能超过 128 个字符'
  if (
    form.membership_group_enabled
    && form.membership_group_replacement_group.trim().length > 128
  ) return '会员权益的套餐互斥组不能超过 128 个字符'
  if (form.usage_policy_enabled) {
    if (form.usage_policies.length === 0) return '至少添加一份使用限制策略'
    if (form.usage_policies.length > MAX_USAGE_POLICY_ENTITLEMENTS) {
      return `每个套餐最多支持 ${MAX_USAGE_POLICY_ENTITLEMENTS} 份使用限制策略`
    }
    if (totalUsagePolicyRules.value > MAX_USAGE_POLICY_TOTAL_RULES) {
      return `每个套餐的使用限制规则合计最多 ${MAX_USAGE_POLICY_TOTAL_RULES} 条`
    }
    for (const [policyIndex, policy] of form.usage_policies.entries()) {
      if (policy.policy_id.trim().length > 128) return `策略 ${policyIndex + 1} 的标识不能超过 128 个字符`
      if (policy.name.trim().length > 128) return `策略 ${policyIndex + 1} 的名称不能超过 128 个字符`
      if (policy.replacement_group.trim().length > 128) return `策略 ${policyIndex + 1} 的套餐互斥组不能超过 128 个字符`
      if (policy.rules.length === 0) return `策略 ${policyIndex + 1} 至少需要一条规则`
      if (policy.rules.length > MAX_USAGE_POLICY_RULES_PER_POLICY) {
        return `策略 ${policyIndex + 1} 最多支持 ${MAX_USAGE_POLICY_RULES_PER_POLICY} 条规则`
      }
      for (const [ruleIndex, rule] of policy.rules.entries()) {
        const label = `策略 ${policyIndex + 1} 的规则 ${ruleIndex + 1}`
        if (!Number.isFinite(Number(rule.limit)) || Number(rule.limit) <= 0) {
          return `${label}上限必须是正数`
        }
        if (rule.metric !== 'actual_cost_usd' && !Number.isInteger(Number(rule.limit))) {
          return `${label}上限必须是正整数`
        }
        if (
          rule.metric !== 'actual_cost_usd'
          && Number(rule.limit) > MAX_USAGE_POLICY_EXACT_INTEGER
        ) {
          return `${label}上限不能超过 ${MAX_USAGE_POLICY_EXACT_INTEGER}`
        }
        if (
          rule.metric === 'actual_cost_usd'
          && (
            Number(rule.limit) < MIN_USAGE_POLICY_COST_USD
            || Number(rule.limit) > MAX_USAGE_POLICY_COST_USD
          )
        ) {
          return `${label}的消费金额上限必须在 ${MIN_USAGE_POLICY_COST_USD} 到 ${MAX_USAGE_POLICY_COST_USD} USD 之间`
        }
        if (rule.metric === 'concurrency' && rule.window_kind !== 'concurrent') {
          return `${label}的并发指标必须使用同时进行窗口`
        }
        if (rule.metric === 'request_count' && rule.window_kind === 'concurrent') {
          return `${label}的请求数指标不能使用同时进行窗口`
        }
        if (rule.metric === 'actual_cost_usd' && rule.window_kind === 'concurrent') {
          return `${label}的消费金额指标不能使用同时进行窗口`
        }
        if (
          rule.window_kind === 'rolling'
          && (!Number.isInteger(Number(rule.rolling_seconds))
            || Number(rule.rolling_seconds) <= 0
            || Number(rule.rolling_seconds) > 2_592_000)
        ) {
          return `${label}的滚动窗口必须是 1 到 2592000 秒之间的整数`
        }
        if (
          rule.window_kind === 'calendar_week'
          && (!Number.isInteger(Number(rule.week_start))
            || Number(rule.week_start) < 1
            || Number(rule.week_start) > 7)
        ) {
          return `${label}的周起始日必须在周一到周日之间`
        }
        if (
          ['calendar_day', 'calendar_week', 'calendar_month'].includes(rule.window_kind)
          && !isValidIanaTimezone(rule.timezone)
        ) {
          return `${label}的时区必须是有效的 IANA 时区，例如 Asia/Shanghai`
        }
      }
    }
  }
  return null
}

function buildPlanPayload(): BillingPlanWriteRequest | null {
  const entitlements = buildEntitlements()
  const validationError = validatePlan(entitlements)
  if (validationError) {
    showError(validationError)
    return null
  }
  return {
    title: form.title.trim(),
    description: form.description.trim() || null,
    price_amount: Number(Number(form.price_amount).toFixed(2)),
    price_currency: form.price_currency.trim().toUpperCase(),
    duration_unit: form.duration_unit,
    duration_value: Number(form.duration_value),
    enabled: form.enabled,
    sort_order: Number(form.sort_order),
    max_active_per_user: showPurchaseLimitCount.value ? Number(form.max_active_per_user) : 1,
    purchase_limit_scope: form.purchase_limit_scope,
    entitlements,
  }
}

async function savePlan() {
  const payload = buildPlanPayload()
  if (!payload) return

  saving.value = true
  try {
    if (props.plan) {
      await adminBillingPlansApi.update(props.plan.id, payload)
      success('套餐已更新')
    } else {
      await adminBillingPlansApi.create(payload)
      success('套餐已创建')
    }
    emit('update:open', false)
    emit('saved')
  } catch (err) {
    log.error('保存套餐失败:', err)
    showError(parseApiError(err, '保存套餐失败'))
  } finally {
    saving.value = false
  }
}

function addManualGroup() {
  const value = manualGroupId.value.trim()
  if (!value) return
  if (!form.grant_user_groups.includes(value)) {
    form.grant_user_groups = [...form.grant_user_groups, value]
  }
  manualGroupId.value = ''
}
</script>
