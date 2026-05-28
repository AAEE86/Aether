<template>
  <div class="space-y-6 pb-8">
    <Tabs v-model="activeTab">
      <TabsList
        class="tabs-button-list grid w-full"
        :class="isAdminPage ? 'max-w-2xl grid-cols-3' : 'max-w-md grid-cols-2'"
      >
        <TabsTrigger value="endpoint">
          端点健康监控
        </TabsTrigger>
        <TabsTrigger value="model">
          模型健康监控
        </TabsTrigger>
        <TabsTrigger
          v-if="isAdminPage"
          value="provider"
        >
          提供商健康监控
        </TabsTrigger>
      </TabsList>

      <TabsContent
        value="endpoint"
        class="mt-4"
      >
        <HealthMonitorCard
          title="端点健康监控"
          :is-admin="isAdminPage"
          :show-provider-info="isAdminPage"
        />
      </TabsContent>

      <TabsContent
        value="model"
        class="mt-4"
      >
        <ModelHealthMonitorCard
          title="模型健康监控"
          :is-admin="isAdminPage"
          :show-provider-info="isAdminPage"
        />
      </TabsContent>

      <TabsContent
        v-if="isAdminPage"
        value="provider"
        class="mt-4"
      >
        <ProviderHealthMonitorCard title="提供商健康监控" />
      </TabsContent>
    </Tabs>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute } from 'vue-router'
import Tabs from '@/components/ui/tabs.vue'
import TabsContent from '@/components/ui/tabs-content.vue'
import TabsList from '@/components/ui/tabs-list.vue'
import TabsTrigger from '@/components/ui/tabs-trigger.vue'
import HealthMonitorCard from '@/features/providers/components/HealthMonitorCard.vue'
import ModelHealthMonitorCard from '@/features/providers/components/ModelHealthMonitorCard.vue'
import ProviderHealthMonitorCard from '@/features/providers/components/ProviderHealthMonitorCard.vue'

const route = useRoute()
const isAdminPage = computed(() => route.path.startsWith('/admin'))
const activeTab = ref('endpoint')
</script>
