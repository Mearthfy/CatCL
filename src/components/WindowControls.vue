<script setup lang="ts">
import { getCurrentWindow } from '@tauri-apps/api/window'
import { desktop } from '../services/backend'
import minimizeIcon from '../assets/figma/minimize.svg'
import closeIcon from '../assets/figma/close.svg'

const emit = defineEmits<{ error: [message: string] }>()
async function windowAction(action: 'minimize' | 'close') {
  if (!desktop) return
  try { await getCurrentWindow()[action]() }
  catch (e) { emit('error', e instanceof Error ? e.message : String(e)) }
}
</script>

<template>
  <div class="window-controls" aria-label="窗口操作">
    <button :disabled="!desktop" aria-label="最小化窗口" :title="desktop ? '最小化' : '桌面版可用'" @click="windowAction('minimize')"><img :src="minimizeIcon" alt="" width="32" height="32" draggable="false"/></button>
    <button :disabled="!desktop" aria-label="关闭窗口" :title="desktop ? '关闭' : '桌面版可用'" @click="windowAction('close')"><img :src="closeIcon" alt="" width="32" height="32" draggable="false"/></button>
  </div>
</template>
