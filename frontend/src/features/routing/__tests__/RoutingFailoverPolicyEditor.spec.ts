import { afterEach, describe, expect, it } from 'vitest'
import { createApp, h, nextTick, ref, type App } from 'vue'
import RoutingFailoverPolicyEditor from '../components/RoutingFailoverPolicyEditor.vue'
import { normalizeRoutingFailoverPolicy, type RoutingFailoverPolicy } from '../utils/routingFailover'

const mounted: Array<{ app: App, root: HTMLElement }> = []

function mountEditor() {
  const policy = ref(normalizeRoutingFailoverPolicy())
  const root = document.createElement('div')
  document.body.appendChild(root)
  const app = createApp({
    setup: () => () => h(RoutingFailoverPolicyEditor, {
      modelValue: policy.value,
      'onUpdate:modelValue': (value: RoutingFailoverPolicy) => { policy.value = value },
    }),
  })
  app.mount(root)
  mounted.push({ app, root })
  return { root, policy }
}

function control<T extends HTMLElement>(root: HTMLElement, label: string): T {
  const element = root.querySelector<T>(`[aria-label="${label}"]`)
  if (!element) throw new Error(`Missing control: ${label}`)
  return element
}

afterEach(() => {
  for (const { app, root } of mounted.splice(0)) {
    app.unmount()
    root.remove()
  }
})

describe('RoutingFailoverPolicyEditor', () => {
  it('edits independent global budgets and documents sticky retry exclusion', async () => {
    const { root, policy } = mountEditor()
    expect(root.textContent).toContain('首次尝试和粘性同 Key 重试不计入')
    expect(root.textContent).toContain('不会中断已开始的调用')
    const count = control<HTMLInputElement>(root, '全局最大转移次数')
    count.value = '4'
    count.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()
    expect(policy.value.max_transfer_count).toBe(4)
    expect(policy.value.max_transfer_timeout_seconds).toBe(0)
  })

  it('adds regex and status-only rules and reports invalid drafts', async () => {
    const { root, policy } = mountEditor()
    control<HTMLButtonElement>(root, '添加成功转移规则').click()
    await nextTick()
    expect(root.querySelector('[role="alert"]')?.textContent).toContain('正则表达式')
    const regex = control<HTMLInputElement>(root, '成功转移规则 1 正则')
    regex.value = '(?i)capacity.*exhausted'
    regex.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()
    expect(policy.value.failover_rules.success_failover_patterns[0].pattern).toBe('(?i)capacity.*exhausted')
    control<HTMLButtonElement>(root, '添加错误提前终止规则').click()
    await nextTick()
    const statuses = control<HTMLInputElement>(root, '终止规则 1 状态码')
    statuses.value = '400, 413'
    statuses.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()
    expect(policy.value.failover_rules.error_stop_patterns[0].status_codes).toEqual([400, 413])
    expect(root.querySelector('[role="alert"]')).toBeNull()
    control<HTMLButtonElement>(root, '删除成功转移规则 1').click()
    await nextTick()
    expect(policy.value.failover_rules.success_failover_patterns).toHaveLength(0)
  })

  it('edits and applies both rule groups through JSON mode', async () => {
    const { root, policy } = mountEditor()
    control<HTMLButtonElement>(root, '切到成功转移规则 JSON').click()
    await nextTick()
    const successJson = root.querySelector<HTMLTextAreaElement>('textarea')
    if (!successJson) throw new Error('Missing success JSON editor')
    successJson.value = '[{"pattern":"capacity"}]'
    successJson.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()
    control<HTMLButtonElement>(root, '切回成功转移规则表单').click()
    await nextTick()
    expect(policy.value.failover_rules.success_failover_patterns).toEqual([{ pattern: 'capacity', status_codes: [] }])

    control<HTMLButtonElement>(root, '切到错误提前终止规则 JSON').click()
    await nextTick()
    const errorJson = root.querySelector<HTMLTextAreaElement>('textarea')
    if (!errorJson) throw new Error('Missing error JSON editor')
    errorJson.value = '[{"status_codes":[429,500],"pattern":"rate"}]'
    errorJson.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()
    control<HTMLButtonElement>(root, '切回错误提前终止规则表单').click()
    await nextTick()
    expect(policy.value.failover_rules.error_stop_patterns).toEqual([{ pattern: 'rate', status_codes: [429, 500] }])
  })

  it('keeps invalid JSON visible until it is corrected', async () => {
    const { root, policy } = mountEditor()
    control<HTMLButtonElement>(root, '切到错误提前终止规则 JSON').click()
    await nextTick()
    const editor = root.querySelector<HTMLTextAreaElement>('textarea')
    if (!editor) throw new Error('Missing JSON editor')
    editor.value = '{'
    editor.dispatchEvent(new Event('input', { bubbles: true }))
    await nextTick()
    control<HTMLButtonElement>(root, '切回错误提前终止规则表单').click()
    await nextTick()
    expect(root.querySelector('[role="alert"]')?.textContent).toContain('JSON')
    expect(policy.value.failover_rules.error_stop_patterns).toHaveLength(0)
  })
})
