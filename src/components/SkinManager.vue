<script setup lang="ts">
import { ImagePlus, Trash2 } from '@lucide/vue'
import { useLauncherStore } from '../stores/launcher'
const store = useLauncherStore()
</script>

<template>
  <section class="panel skin-panel">
    <div class="section-heading"><h2><ImagePlus :size="20"/>玩家基础皮肤</h2><span>{{ store.settings.offlineName }}</span></div>
    <div class="skin-content">
      <div class="skin-head" aria-label="皮肤头像预览"><img v-if="store.skin" :src="store.skin.dataUrl" alt="玩家皮肤头像"/><span v-else>?</span></div>
      <div class="skin-copy"><strong>{{ store.skin ? `${store.skin.width}×${store.skin.height} PNG` : '尚未导入皮肤' }}</strong><p>皮肤绑定当前离线玩家名；支持标准 64×64 和旧版 64×32 PNG。</p></div>
      <button class="button secondary" :disabled="!store.desktop || store.gameRunning" @click="store.importSkin"><ImagePlus :size="16"/>选择 PNG</button>
      <button v-if="store.skin" class="button secondary" :disabled="store.gameRunning" @click="store.removeSkin"><Trash2 :size="16"/>移除</button>
    </div>
  </section>
</template>
