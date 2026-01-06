import { createRouter, createWebHistory } from 'vue-router'
import type { RouteRecordRaw } from 'vue-router'
import { useAuthStore } from '@/stores/auth'
import { importWithRetry } from '@/utils/importRetry'
import { log } from '@/utils/logger'

const routes: RouteRecordRaw[] = [
  {
    path: '/',
    name: 'Home',
    component: () => importWithRetry(() => import('@/views/public/Home.vue')),
    meta: { requiresAuth: false }
  },

  {
    path: '/logo-demo',
    name: 'LogoColorDemo',
    component: () => importWithRetry(() => import('@/views/public/LogoColorDemo.vue')),
    meta: { requiresAuth: false }
  },

  {
    path: '/dashboard',
    component: () => importWithRetry(() => import('@/layouts/MainLayout.vue')),
    meta: { requiresAuth: true },
    children: [
      {
        path: '',
        name: 'Dashboard',
        component: () => importWithRetry(() => import('@/views/shared/Dashboard.vue'))
      },
      {
        path: 'api-keys',
        name: 'MyApiKeys',
        component: () => importWithRetry(() => import('@/views/user/MyApiKeys.vue'))
      },
      {
        path: 'announcements',
        name: 'Announcements',
        component: () => importWithRetry(() => import('@/views/user/Announcements.vue'))
      },
      {
        path: 'usage',
        name: 'MyUsage',
        component: () => importWithRetry(() => import('@/views/shared/Usage.vue'))
      },
      {
        path: 'endpoint-status',
        name: 'EndpointStatus',
        component: () => importWithRetry(() => import('@/views/shared/HealthMonitor.vue'))
      },
      {
        path: 'settings',
        name: 'Settings',
        component: () => importWithRetry(() => import('@/views/user/Settings.vue'))
      },
      {
        path: 'models',
        name: 'ModelCatalog',
        component: () => importWithRetry(() => import('@/views/user/ModelCatalog.vue'))
      }
    ]
  },
  {
    path: '/admin',
    component: () => importWithRetry(() => import('@/layouts/MainLayout.vue')),
    meta: { requiresAuth: true, requiresAdmin: true },
    children: [
      {
        path: 'dashboard',
        name: 'AdminDashboard',
        component: () => importWithRetry(() => import('@/views/shared/Dashboard.vue'))
      },
      {
        path: 'users',
        name: 'Users',
        component: () => importWithRetry(() => import('@/views/admin/Users.vue'))
      },
      {
        path: 'keys',
        name: 'ApiKeys',
        component: () => importWithRetry(() => import('@/views/admin/ApiKeys.vue'))
      },
      {
        path: 'providers',
        name: 'ProviderManagement',
        component: () => importWithRetry(() => import('@/views/admin/ProviderManagement.vue'))
      },
      {
        path: 'models',
        name: 'ModelManagement',
        component: () => importWithRetry(() => import('@/views/admin/ModelManagement.vue'))
      },
      {
        path: 'health-monitor',
        name: 'HealthMonitor',
        component: () => importWithRetry(() => import('@/views/shared/HealthMonitor.vue'))
      },
      {
        path: 'usage',
        name: 'Usage',
        component: () => importWithRetry(() => import('@/views/shared/Usage.vue'))
      },
      {
        path: 'system',
        name: 'SystemSettings',
        component: () => importWithRetry(() => import('@/views/admin/SystemSettings.vue'))
      },
      {
        path: 'email',
        name: 'EmailSettings',
        component: () => importWithRetry(() => import('@/views/admin/EmailSettings.vue'))
      },
      {
        path: 'ldap',
        name: 'LdapSettings',
        component: () => importWithRetry(() => import('@/views/admin/LdapSettings.vue'))
      },
      {
        path: 'audit-logs',
        name: 'AuditLogs',
        component: () => importWithRetry(() => import('@/views/admin/AuditLogs.vue'))
      },
      {
        path: 'cache-monitoring',
        name: 'CacheMonitoring',
        component: () => importWithRetry(() => import('@/views/admin/CacheMonitoring.vue'))
      },
      {
        path: 'ip-security',
        name: 'IPSecurity',
        component: () => importWithRetry(() => import('@/views/admin/IPSecurity.vue'))
      },
      {
        path: 'announcements',
        name: 'AnnouncementManagement',
        component: () => importWithRetry(() => import('@/views/user/Announcements.vue'))
      }
    ]
  }
]

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes
})

/**
 * 判断错误是否为网络错误
 */
function isNetworkError(error: any): boolean {
  return !error.response || error.message?.includes('Network') || error.message?.includes('timeout')
}

router.beforeEach(async (to, from, next) => {
  const authStore = useAuthStore()

  try {
    // 如果有token但没有用户信息,尝试获取用户信息
    if (authStore.token && !authStore.user) {
      try {
        await authStore.fetchCurrentUser()
      } catch (error: any) {
        // 区分网络错误和认证错误
        if (isNetworkError(error)) {
          log.warn('Network error while fetching user info, keeping session', { error: error?.message })
        } else if (error.response?.status === 401) {
          log.info('Authentication failed, clearing session')
          authStore.logout()
        } else {
          log.warn('Failed to fetch user info, but keeping session', { error: error?.message })
        }
      }
    }

    // 检查整个路由匹配记录链中的 meta
    const requiresAuth = to.matched.some(record => record.meta.requiresAuth !== false)
    const requiresAdmin = to.matched.some(record => record.meta.requiresAdmin)

    // 如果需要认证但没有token,跳转到首页
    if (requiresAuth && !authStore.token) {
      sessionStorage.setItem('redirectPath', to.fullPath)
      log.debug('No valid token found, redirecting to home')
      next('/')
    } else if (to.path === '/' && authStore.isAuthenticated && (to.query.returnTo || from.path.startsWith('/dashboard') || from.path.startsWith('/admin') || from.path === '/')) {
      // 已登录用户如果是从dashboard返回首页、刷新首页、或者有returnTo参数,允许访问首页
      next()
    } else if (authStore.isAuthenticated && to.path === '/' && !to.query.returnTo) {
      // 已登录用户首次访问首页(非返回/刷新场景),根据角色跳转到对应仪表盘
      const isAdmin = authStore.user?.role === 'admin'
      const redirectPath = sessionStorage.getItem('redirectPath')
      if (redirectPath && redirectPath !== '/') {
        sessionStorage.removeItem('redirectPath')
        next(redirectPath)
      } else {
        next(isAdmin ? '/admin/dashboard' : '/dashboard')
      }
    } else if (requiresAdmin) {
      // 需要管理员权限的页面
      const isAdmin = authStore.user?.role === 'admin'
      if (!isAdmin) {
        log.warn('Non-admin user attempted to access admin page, redirecting to user dashboard')
        next('/dashboard')
      } else {
        next()
      }
    } else {
      next()
    }
  } catch (error) {
    log.error('Router guard error', error)
    // 发生错误时,直接放行,不要乱跳转
    next()
  }
})

export default router
