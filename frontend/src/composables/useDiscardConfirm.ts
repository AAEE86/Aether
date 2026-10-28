import { useConfirm } from './useConfirm'

/**
 * 关闭表单前的放弃确认
 *
 * 当表单存在未保存更改时弹出警告确认框，否则直接放行。
 */
export function useDiscardConfirm(isDirty: () => boolean) {
  const { confirmWarning } = useConfirm()

  async function confirmDiscard(): Promise<boolean> {
    if (!isDirty()) return true
    return confirmWarning('有未保存的更改，确定要关闭吗？', '放弃更改')
  }

  return { confirmDiscard }
}
