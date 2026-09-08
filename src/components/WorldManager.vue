<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { Box, FolderOpen, Pencil, Plus, RefreshCw, Search, X } from '@lucide/vue'
import { backend } from '../services/backend'
import { useLauncherStore } from '../stores/launcher'
import type { WorldInfo } from '../types'

const launcher = useLauncherStore()
const worlds = ref<WorldInfo[]>([])
const warnings = ref<string[]>([])
const search = ref('')
const loading = ref(false)
const busy = ref(false)
const modal = ref<'import' | 'rename' | null>(null)
const selected = ref<WorldInfo | null>(null)
const name = ref('')
const source = ref('')
const formError = ref('')
const error = ref('')
const notice = ref('')
const dialog = ref<HTMLDialogElement | null>(null)
const gameDirectory = computed(() => launcher.selectedInstance ? `${launcher.selectedInstance.path.replace(/[\\/]+$/, '')}\\game` : '')
watch(modal, async value => {
  await nextTick()
  if (value) { dialog.value?.showModal(); dialog.value?.querySelector<HTMLInputElement>('input')?.focus() }
  else dialog.value?.close()
})
const filtered = computed(() => worlds.value.filter(w => `${w.name} ${w.folder} ${w.location}`.toLocaleLowerCase().includes(search.value.toLocaleLowerCase())))
const message = (e: unknown) => e instanceof Error ? e.message : String(e)
let generation = 0
async function refresh() {
  const run = ++generation
  const directory = gameDirectory.value
  worlds.value = []; warnings.value = []; error.value = ''
  if (!launcher.desktop || !directory) return
  loading.value = true
  try {
    const library = await backend.worlds(directory)
    if (run === generation) { worlds.value = library.worlds; warnings.value = library.warnings }
  } catch (e) { if (run === generation) error.value = message(e) }
  finally { if (run === generation) loading.value = false }
}
watch(gameDirectory, refresh, { immediate: true })
function edit(world?: WorldInfo) {
  selected.value = world ?? null
  name.value = world?.name ?? ''
  source.value = ''; formError.value = ''; notice.value = ''
  modal.value = world ? 'rename' : 'import'
}
async function browseSource() {
  try {
    const path = await backend.pickDirectory(source.value || undefined)
    if (path) source.value = path
  } catch (e) { formError.value = message(e) }
}
async function submit() {
  if (busy.value) return
  const trimmed = name.value.trim()
  if (!trimmed || [...trimmed].length > 80 || /[\u0000-\u001f\u007f]/.test(trimmed)) { formError.value = '请输入 1–80 个字符的名称，不要包含换行。'; return }
  if (modal.value === 'import' && !source.value.trim()) { formError.value = '请填写已解压世界的文件夹路径。'; return }
  busy.value = true; formError.value = ''
  try {
    const directory = gameDirectory.value
    if (modal.value === 'rename' && selected.value) await backend.renameWorld(directory, selected.value.id, trimmed)
    else await backend.importWorld(directory, source.value.trim(), trimmed)
    notice.value = modal.value === 'rename' ? '世界名称已保存' : '世界已导入，原文件保留'
    modal.value = null
    await refresh()
  } catch (e) { formError.value = message(e) }
  finally { busy.value = false }
}
async function open(world: WorldInfo) { try { await backend.openWorld(gameDirectory.value, world.id) } catch (e) { error.value = message(e) } }
</script>

<template>
  <div class="world-manager">
    <div class="toolbar"><label class="search-field"><Search :size="17"/><input v-model="search" placeholder="搜索世界名称…" aria-label="搜索世界"/></label><button class="button secondary" :disabled="!launcher.desktop || loading || busy" @click="refresh"><RefreshCw :size="16" :class="{ spinning: loading }"/>刷新</button><button class="button primary" :disabled="!launcher.desktop || !launcher.selectedInstance || busy" @click="edit()"><Plus :size="16"/>导入世界</button></div>
    <div class="section-heading"><h2>我的世界 <span class="count">{{ worlds.length }}</span></h2><span>本地地图与存档</span></div>
    <div v-if="error" class="banner error" role="alert">{{ error }}</div>
    <div v-if="notice" class="banner success" role="status">{{ notice }}</div>
    <div v-for="warning in warnings" :key="warning" class="banner error">{{ warning }}</div>
    <div v-if="loading" class="empty-state"><RefreshCw class="spinning" :size="30"/><h3>正在查找本地世界</h3></div>
    <div v-else-if="!filtered.length" class="empty-state"><Box :size="36"/><h3>{{ worlds.length ? '没有匹配的世界' : '还没有世界存档' }}</h3><p>{{ launcher.desktop ? '可以导入下载并解压的地图，或刷新已有 saves 存档。' : '打开桌面版，即可导入和管理本地世界。' }}</p></div>
    <div v-else class="world-list"><article v-for="world in filtered" :key="world.id" class="world-card"><div class="block-icon"><Box :size="25"/></div><div class="world-info"><h3>{{ world.name }}</h3><p>{{ world.location }} · {{ world.modifiedAt ? new Date(world.modifiedAt).toLocaleDateString('zh-CN') : '未知修改时间' }}</p><span :title="world.id">{{ world.folder }}</span></div><div class="world-actions"><button class="button secondary" :aria-label="`重命名 ${world.name}`" @click="edit(world)"><Pencil :size="14"/>命名</button><button class="button secondary" :aria-label="`打开 ${world.name} 的文件夹`" @click="open(world)"><FolderOpen :size="14"/>文件夹</button></div></article></div>
    <p class="field-help world-help">名称保存在启动器的世界标签中；游戏内名称和存档文件夹不变。扫描当前游戏目录的 saves，以及 instances / versions 下的独立存档。</p>
    <Teleport to="body"><dialog ref="dialog" class="world-dialog" aria-labelledby="world-dialog-title" @cancel.prevent="!busy && (modal = null)"><header><h2 id="world-dialog-title">{{ modal === 'rename' ? '给世界起个名字' : '导入本地世界' }}</h2><button class="icon-button" :disabled="busy" aria-label="关闭对话框" @click="modal = null"><X :size="19"/></button></header><form @submit.prevent="submit"><label for="world-name">世界显示名称</label><input id="world-name" v-model="name" :disabled="busy" maxlength="160" placeholder="例如：我们的生存小镇" autofocus/><template v-if="modal === 'import'"><label for="world-source">已解压的世界文件夹</label><div class="input-action"><input id="world-source" v-model="source" :disabled="busy" placeholder="D:\下载\我的地图"/><button type="button" class="button secondary" :disabled="busy || !launcher.desktop" @click="browseSource"><FolderOpen :size="15"/>浏览</button></div><p class="field-help">请选择直接包含 level.dat 的文件夹。ZIP 地图请先解压，导入会复制完整存档，保留原文件。</p></template><p v-else class="field-help">可使用中文；只修改启动器中的显示名称。</p><p v-if="formError" class="banner error" role="alert">{{ formError }}</p><div class="dialog-actions"><button type="button" class="button secondary" :disabled="busy" @click="modal = null">取消</button><button class="button primary" :disabled="busy" type="submit">{{ busy ? '正在处理…' : '保存' }}</button></div></form></dialog></Teleport>
  </div>
</template>
