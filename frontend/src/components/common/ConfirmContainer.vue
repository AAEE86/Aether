<template>
  <AlertDialog
    :model-value="state.isOpen"
    :title="state.title || '确认操作'"
    :description="state.message"
    :confirm-text="state.confirmText || '确认'"
    :cancel-text="state.cancelText || '取消'"
    :type="state.variant || 'question'"
    @update:model-value="handleClose"
    @confirm="handleConfirm"
    @cancel="handleCancel"
  />
</template>

<script setup lang="ts">
import AlertDialog from './AlertDialog.vue'
import { useConfirm } from '@/composables/useConfirm'

const { state, handleConfirm, handleCancel } = useConfirm()

function handleClose(value: boolean) {
  if (!value) {
    handleCancel()
  }
}
</script>