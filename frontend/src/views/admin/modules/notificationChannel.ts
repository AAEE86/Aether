import { getI18nLocale } from '@/i18n'

const CHANNEL_LABELS_ZH: Record<string, string> = {
  bark: 'Bark',
  server_chan: 'Server 酱',
  email: '邮件',
  user_email: '用户邮件',
  module: '模块',
  item: '通知项',
  none: '无可用服务',
}

const CHANNEL_LABELS_EN: Record<string, string> = {
  bark: 'Bark',
  server_chan: 'ServerChan',
  email: 'Email',
  user_email: 'User email',
  module: 'Module',
  item: 'Notification item',
  none: 'No service available',
}

export function formatNotificationChannel(channel: string): string {
  const labels = getI18nLocale() === 'en-US' ? CHANNEL_LABELS_EN : CHANNEL_LABELS_ZH
  return labels[channel] ?? channel
}
