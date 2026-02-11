import { ref, computed, watch, nextTick, onUnmounted, type Ref, type ComputedRef } from 'vue'

/**
 * 智能分页 composable
 *
 * 根据列表容器的实际渲染高度自动决定是否分页以及每页条数。
 * 当列表总高度超过阈值时自动启用分页，低于阈值时恢复全量显示。
 * 使用 ResizeObserver 确保 DOM 稳定后再进行检测。
 *
 * @param items        响应式数据源（全量列表）
 * @param listRef      列表容器 DOM 引用
 * @param maxHeight    触发分页的高度阈值（px），默认 500
 */
export function useSmartPagination<T>(
  items: ComputedRef<T[]> | Ref<T[]>,
  listRef: Ref<HTMLElement | null>,
  maxHeight = 500,
) {
  const currentPage = ref(1)
  const itemsPerPage = ref(0) // 0 = 不分页
  const cachedAvgItemHeight = ref(0)

  let resizeObserver: ResizeObserver | null = null
  let pendingDetect = false
  let detectDebounceTimer: ReturnType<typeof setTimeout> | null = null

  const shouldPaginate = computed(() => {
    return itemsPerPage.value > 0 && items.value.length > itemsPerPage.value
  })

  const paginatedItems = computed(() => {
    if (!shouldPaginate.value) return items.value
    const start = (currentPage.value - 1) * itemsPerPage.value
    return items.value.slice(start, start + itemsPerPage.value)
  })

  const totalPages = computed(() => {
    if (!shouldPaginate.value) return 1
    return Math.ceil(items.value.length / itemsPerPage.value)
  })

  /** 分页激活时的固定高度（px），用于防止翻页时容器高度跳动 */
  const fixedHeight = computed(() => {
    if (!shouldPaginate.value || cachedAvgItemHeight.value <= 0) return undefined
    return itemsPerPage.value * cachedAvgItemHeight.value
  })

  /** 将当前页内的局部索引转换为全局索引 */
  function getGlobalIndex(localIdx: number): number {
    if (!shouldPaginate.value) return localIdx
    return (currentPage.value - 1) * itemsPerPage.value + localIdx
  }

  /** 检测是否需要分页并计算每页条数 */
  function detect() {
    const el = listRef.value
    if (!el || items.value.length <= 2) {
      itemsPerPage.value = 0
      return
    }

    const scrollHeight = el.scrollHeight
    const renderedCount = paginatedItems.value.length

    // 确保 DOM 已渲染且有有效高度
    if (scrollHeight === 0 || renderedCount === 0) {
      return
    }

    cachedAvgItemHeight.value = scrollHeight / renderedCount

    const estimatedTotalHeight = cachedAvgItemHeight.value * items.value.length
    if (estimatedTotalHeight > maxHeight && cachedAvgItemHeight.value > 0) {
      itemsPerPage.value = Math.max(Math.floor(maxHeight / cachedAvgItemHeight.value), 3)
      const maxPage = Math.ceil(items.value.length / itemsPerPage.value)
      if (currentPage.value > maxPage) {
        currentPage.value = maxPage
      }
    } else {
      itemsPerPage.value = 0
    }
  }

  /** 防抖检测，避免频繁触发 */
  function debouncedDetect() {
    if (detectDebounceTimer) {
      clearTimeout(detectDebounceTimer)
    }
    detectDebounceTimer = setTimeout(() => {
      detect()
      detectDebounceTimer = null
    }, 50)
  }

  /** 设置 ResizeObserver 监听 DOM 变化 */
  function setupResizeObserver() {
    cleanupResizeObserver()

    const el = listRef.value
    if (!el) return

    resizeObserver = new ResizeObserver(() => {
      // DOM 尺寸变化时重新检测
      if (pendingDetect) {
        pendingDetect = false
        debouncedDetect()
      }
    })

    resizeObserver.observe(el)
  }

  /** 清理 ResizeObserver */
  function cleanupResizeObserver() {
    if (resizeObserver) {
      resizeObserver.disconnect()
      resizeObserver = null
    }
    if (detectDebounceTimer) {
      clearTimeout(detectDebounceTimer)
      detectDebounceTimer = null
    }
  }

  /** 重置分页状态（数据源切换时调用） */
  function reset() {
    currentPage.value = 1
    itemsPerPage.value = 0
    cachedAvgItemHeight.value = 0
  }

  /** 请求检测（会等待 DOM 稳定后执行） */
  function requestDetect() {
    pendingDetect = true

    // 先尝试立即检测（快路径），统一走防抖避免与 ResizeObserver 双重触发
    nextTick(() => {
      const el = listRef.value
      if (el && el.scrollHeight > 0) {
        // DOM 已就绪，通过防抖检测
        pendingDetect = false
        debouncedDetect()
      }
      // 否则等待 ResizeObserver 回调
    })
  }

  // 监听 listRef 变化，设置/更新 ResizeObserver
  watch(listRef, (newEl) => {
    if (newEl) {
      setupResizeObserver()
      requestDetect()
    } else {
      cleanupResizeObserver()
    }
  }, { immediate: true })

  // 数据源变化时重置页码并请求检测
  watch(items, () => {
    currentPage.value = 1
    requestDetect()
  }, { flush: 'post' })

  // 组件卸载时清理
  onUnmounted(() => {
    cleanupResizeObserver()
  })

  return {
    currentPage,
    totalPages,
    shouldPaginate,
    paginatedItems,
    fixedHeight,
    getGlobalIndex,
    detect,
    reset,
  }
}
