<template>
  <PageContainer padding="lg">
    <PageHeader
      title="套餐管理"
      description="配置额度、流量限制、会员权益和组合套餐"
    >
      <template #actions>
        <Button
          size="sm"
          @click="openCreateDialog"
        >
          <Plus class="mr-2 h-4 w-4" />
          新建套餐
        </Button>
      </template>
    </PageHeader>

    <div class="mt-6 space-y-6">
      <div
        v-if="loading"
        class="py-16"
      >
        <LoadingState message="正在加载套餐..." />
      </div>

      <CardSection
        v-else
        title="套餐列表"
        description="启用后的套餐会出现在用户套餐中心"
      >
        <div
          v-if="plans.length === 0"
          class="py-12"
        >
          <EmptyState
            title="暂无套餐"
            description="创建第一个套餐后，用户可以在套餐中心购买"
          />
        </div>

        <div
          v-else
          class="overflow-x-auto"
        >
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead class="w-[24%]">
                  套餐
                </TableHead>
                <TableHead class="w-[10%] whitespace-nowrap">
                  价格
                </TableHead>
                <TableHead class="w-[18%] whitespace-nowrap">
                  权益有效期
                </TableHead>
                <TableHead class="w-[20%]">
                  权益
                </TableHead>
                <TableHead class="w-[6%] whitespace-nowrap text-center">
                  排序
                </TableHead>
                <TableHead class="w-[7%] whitespace-nowrap text-center">
                  状态
                </TableHead>
                <TableHead class="w-[15%] whitespace-nowrap text-right">
                  操作
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="plan in plans"
                :key="plan.id"
              >
                <TableCell>
                  <div class="text-sm font-medium">
                    {{ plan.title }}
                  </div>
                  <div
                    v-if="plan.description"
                    class="mt-0.5 max-w-[280px] truncate text-xs text-muted-foreground"
                  >
                    {{ plan.description }}
                  </div>
                </TableCell>
                <TableCell class="whitespace-nowrap text-sm tabular-nums">
                  {{ formatPlanPriceAmount(plan) }} {{ plan.price_currency }}
                </TableCell>
                <TableCell>
                  <div class="whitespace-nowrap text-sm">
                    {{ formatPlanPeriod(plan) }}
                  </div>
                  <div class="mt-0.5 text-xs text-muted-foreground">
                    {{ planDurationHint(plan) }} · {{ planPurchaseLimitHint(plan) }}
                  </div>
                </TableCell>
                <TableCell>
                  <div class="flex flex-wrap items-center gap-1">
                    <span
                      v-for="item in entitlementBadges(plan)"
                      :key="item"
                      class="inline-flex whitespace-nowrap rounded-md border border-border/60 bg-muted/40 px-2 py-0.5 text-xs"
                    >
                      {{ item }}
                    </span>
                  </div>
                </TableCell>
                <TableCell class="text-center text-sm tabular-nums text-muted-foreground">
                  {{ plan.sort_order }}
                </TableCell>
                <TableCell class="text-center">
                  <span
                    class="inline-flex items-center gap-1.5 text-xs"
                    :class="plan.enabled ? 'text-emerald-500' : 'text-muted-foreground'"
                  >
                    <span
                      class="h-1.5 w-1.5 rounded-full"
                      :class="plan.enabled ? 'bg-emerald-500' : 'bg-muted-foreground/40'"
                    />
                    {{ plan.enabled ? '已启用' : '已停用' }}
                  </span>
                </TableCell>
                <TableCell class="whitespace-nowrap text-right">
                  <div class="inline-flex items-center">
                    <Button
                      variant="ghost"
                      size="sm"
                      :disabled="deletingPlanId === plan.id"
                      @click="togglePlanStatus(plan)"
                    >
                      {{ plan.enabled ? '停用' : '启用' }}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      :disabled="deletingPlanId === plan.id"
                      @click="openEditDialog(plan)"
                    >
                      编辑
                    </Button>
                    <DropdownMenu>
                      <DropdownMenuTrigger as-child>
                        <Button
                          variant="ghost"
                          size="sm"
                          class="h-9 w-9 p-0"
                          :disabled="deletingPlanId === plan.id"
                        >
                          <MoreHorizontal class="h-4 w-4" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem
                          class="text-destructive focus:text-destructive"
                          :disabled="deletingPlanId === plan.id"
                          @select="deletePlan(plan)"
                        >
                          <Trash2 class="mr-2 h-4 w-4" />
                          {{ deletingPlanId === plan.id ? '删除中...' : '删除套餐' }}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </div>
      </CardSection>
    </div>

    <BillingPlanFormDialog
      :open="dialogOpen"
      :plan="editingPlan"
      :user-groups="userGroups"
      @update:open="dialogOpen = $event"
      @saved="loadPlans"
    />
  </PageContainer>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { MoreHorizontal, Plus, Trash2 } from 'lucide-vue-next'
import {
  adminBillingPlansApi,
  type BillingPlan,
} from '@/api/billing'
import { usersApi, type UserGroup } from '@/api/users'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui'
import { EmptyState, LoadingState } from '@/components/common'
import { CardSection, PageContainer, PageHeader } from '@/components/layout'
import { useToast } from '@/composables/useToast'
import { useI18n } from '@/i18n'
import { parseApiError } from '@/utils/errorParser'
import { log } from '@/utils/logger'
import { formatBillingPlanDuration } from '@/utils/billingEntitlements'
import BillingPlanFormDialog from '@/features/billing/components/BillingPlanFormDialog.vue'
import { formatUsagePolicyBadge, resolvePlanModeFromEntitlements } from '@/features/billing/utils/billingPlanForm'

const { success, error: showError } = useToast()
const { legacyT } = useI18n()

const loading = ref(true)
const deletingPlanId = ref<string | null>(null)
const dialogOpen = ref(false)
const plans = ref<BillingPlan[]>([])
const editingPlan = ref<BillingPlan | null>(null)
const userGroups = ref<UserGroup[]>([])

onMounted(() => {
  void Promise.all([loadPlans(), loadUserGroups()]).finally(() => {
    loading.value = false
  })
})

async function loadPlans() {
  try {
    const response = await adminBillingPlansApi.list()
    plans.value = [...response.items].sort((left, right) =>
      left.sort_order === right.sort_order
        ? left.price_amount - right.price_amount
        : left.sort_order - right.sort_order
    )
  } catch (err) {
    log.error('加载套餐失败:', err)
    showError(parseApiError(err, '加载套餐失败'))
  }
}

async function loadUserGroups() {
  try {
    const response = await usersApi.listUserGroups()
    userGroups.value = response.items
  } catch (err) {
    log.error('加载用户分组失败:', err)
    showError(parseApiError(err, '加载用户分组失败'))
  }
}

function openCreateDialog() {
  editingPlan.value = null
  dialogOpen.value = true
}

function openEditDialog(plan: BillingPlan) {
  editingPlan.value = plan
  dialogOpen.value = true
}

async function togglePlanStatus(plan: BillingPlan) {
  try {
    await adminBillingPlansApi.setStatus(plan.id, !plan.enabled)
    success(plan.enabled ? '套餐已停用' : '套餐已启用')
    await loadPlans()
  } catch (err) {
    log.error('更新套餐状态失败:', err)
    showError(parseApiError(err, '更新套餐状态失败'))
  }
}

async function deletePlan(plan: BillingPlan) {
  if (deletingPlanId.value) return
  const confirmed = window.confirm(
    legacyT(`确定删除套餐「${plan.title}」吗？\n\n已有订单或权益的套餐不能删除，请改为停用。删除后无法恢复。`)
  )
  if (!confirmed) return

  deletingPlanId.value = plan.id
  try {
    await adminBillingPlansApi.delete(plan.id)
    success('套餐已删除')
    await loadPlans()
  } catch (err) {
    log.error('删除套餐失败:', err)
    showError(parseApiError(err, '删除套餐失败'))
  } finally {
    deletingPlanId.value = null
  }
}

function formatPlanPriceAmount(plan: BillingPlan): string {
  return Number(plan.price_amount || 0).toFixed(2)
}

function formatPlanPeriod(plan: BillingPlan): string {
  return formatBillingPlanDuration(plan.duration_unit, plan.duration_value)
}

function planPurchaseLimitHint(plan: BillingPlan): string {
  if (plan.purchase_limit_scope === 'unlimited') return '不限购'
  if (plan.purchase_limit_scope === 'lifetime') {
    return `每人最多购买 ${plan.max_active_per_user} 次`
  }
  return `最多同时生效 ${plan.max_active_per_user} 份`
}

function planDurationHint(plan: BillingPlan): string {
  if (plan.purchase_limit_scope === 'unlimited') return '不限制重复购买'
  if (plan.purchase_limit_scope === 'lifetime') return '不按周期重置购买次数'
  const mode = resolvePlanModeFromEntitlements(plan.entitlements)
  if (mode === 'wallet') return '旧余额套餐，建议停用'
  if (mode === 'daily') return '每日额度周期'
  if (mode === 'membership') return '会员权限周期'
  if (mode === 'usage') return '使用限制周期'
  if (mode === 'mixed') return '组合权益周期'
  return '未配置权益'
}

function groupName(groupId: string): string {
  return userGroups.value.find((group) => group.id === groupId)?.name || groupId
}

function entitlementBadges(plan: BillingPlan): string[] {
  return (plan.entitlements || []).map((entitlement) => {
    if (entitlement.type === 'wallet_credit') {
      return `附赠余额 $${Number(entitlement.amount_usd || 0).toFixed(2)}`
    }
    if (entitlement.type === 'daily_quota') {
      return `每日 $${Number(entitlement.daily_quota_usd || 0).toFixed(2)}`
    }
    if (entitlement.type === 'membership_group') {
      const groups = entitlement.grant_user_groups.map(groupName).join(', ')
      return `会员组 ${groups}`
    }
    if (entitlement.type === 'usage_policy') {
      return formatUsagePolicyBadge(entitlement)
    }
    return '未知权益'
  })
}

</script>
