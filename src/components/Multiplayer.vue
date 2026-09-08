<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { Check, Clipboard, Copy, Link2, LoaderCircle, Radio, ShieldCheck, Unplug, Users } from '@lucide/vue'
import { backend } from '../services/backend'
import type { GameInstance, P2pRoomSnapshot } from '../types'

const props = defineProps<{ instance: GameInstance | null }>()
const room = ref<P2pRoomSnapshot | null>(null)
const inviteInput = ref('')
const answerInput = ref('')
const busy = ref(false)
const error = ref('')
const copied = ref('')
let unlisten: (() => void) | undefined

const active = computed(() => room.value && !['Idle', 'Closed', 'Failed'].includes(room.value.state))
async function run(action: () => Promise<P2pRoomSnapshot>) {
  busy.value = true; error.value = ''
  try { room.value = await action() } catch (reason) { error.value = String(reason) } finally { busy.value = false }
}
async function copy(value: string | null, label: string) {
  if (!value) return
  try { await navigator.clipboard.writeText(value); copied.value = label; setTimeout(() => { copied.value = '' }, 1500) } catch { error.value = '无法访问剪贴板，请手动选择并复制。' }
}
function createRoom() {
  if (!props.instance) { error.value = '请先选择一个已安装实例。'; return }
  run(() => backend.createP2pRoom(props.instance!.path))
}
function joinRoom() { if (inviteInput.value.trim()) run(() => backend.joinP2pRoom(inviteInput.value.trim())) }
function acceptAnswer() { if (answerInput.value.trim()) run(() => backend.acceptP2pAnswer(answerInput.value.trim())) }
function connectRoom() { run(() => backend.connectP2pRoom()) }
function closeRoom() { run(() => backend.closeP2pRoom()) }

onMounted(async () => {
  if (backend && 'onP2pRoomState' in backend) unlisten = await backend.onP2pRoomState(status => { room.value = status })
  try { room.value = await backend.p2pRoomStatus() } catch { /* browser preview */ }
})
onBeforeUnmount(() => unlisten?.())
</script>

<template>
  <section class="multiplayer-intro">
    <div><span class="pill"><Radio :size="14"/> CCL ENCRYPTED P2P</span><h2>和朋友直接连接</h2><p>无需虚拟网卡。Minecraft 连接本机地址，CCL 通过加密 QUIC 通道转发到房主的局域网世界。</p></div>
    <ShieldCheck :size="58"/>
  </section>
  <div v-if="error" class="banner error" role="alert">{{ error }}</div>
  <section v-if="!active" class="multiplayer-grid">
    <article class="panel room-action"><Users :size="28"/><h3>创建联机房间</h3><p>先在 Minecraft 中点击“对局域网开放”，CCL 会自动发现端口并生成邀请码。</p><strong>{{ instance ? `${instance.name} · ${instance.version}` : '尚未选择实例' }}</strong><button class="button primary" :disabled="busy || !instance" @click="createRoom"><LoaderCircle v-if="busy" class="spinning" :size="16"/><Radio v-else :size="16"/>创建房间</button></article>
    <article class="panel room-action"><Link2 :size="28"/><h3>加入联机房间</h3><p>粘贴房主发送的 CCL 邀请码，生成只属于本次连接的回应码。</p><textarea v-model.trim="inviteInput" spellcheck="false" placeholder="CCL://…"/><button class="button primary" :disabled="busy || !inviteInput" @click="joinRoom"><LoaderCircle v-if="busy" class="spinning" :size="16"/><Clipboard v-else :size="16"/>解析邀请码</button></article>
  </section>
  <section v-else class="panel room-console">
    <div class="section-heading"><h2><Radio :size="19"/>{{ room?.role === 'host' ? '房主房间' : '加入房间' }}</h2><span>{{ room?.state }}</span></div>
    <div class="network-facts"><span>状态<strong>{{ room?.message }}</strong></span><span>传输<strong>{{ room?.transport || '检测中' }}</strong></span><span>玩家<strong>{{ room?.playerCount || 1 }} / 4</strong></span><span>LAN 端口<strong>{{ room?.minecraftLanPort || '—' }}</strong></span></div>
    <template v-if="room?.role === 'host'">
      <label>房主邀请码<textarea :value="room.inviteCode || ''" readonly/><button class="button secondary" @click="copy(room.inviteCode, '邀请码')"><Check v-if="copied === '邀请码'" :size="16"/><Copy v-else :size="16"/>复制邀请码</button></label>
      <label>好友回应码<textarea v-model.trim="answerInput" placeholder="粘贴好友发回的 CCL:// 回应码"/><button class="button primary" :disabled="busy || !answerInput" @click="acceptAnswer"><LoaderCircle v-if="busy" class="spinning" :size="16"/>等待好友连接</button></label>
      <p class="field-help">点击后请让好友立即点击“建立连接”，双方会使用同一 UDP 端口完成打洞。</p>
    </template>
    <template v-else>
      <label>回应码<textarea :value="room?.answerCode || ''" readonly/><button class="button secondary" @click="copy(room?.answerCode || null, '回应码')"><Check v-if="copied === '回应码'" :size="16"/><Copy v-else :size="16"/>复制回应码</button></label>
      <button v-if="!room?.localAddress" class="button primary connect-room" :disabled="busy" @click="connectRoom"><LoaderCircle v-if="busy" class="spinning" :size="16"/><Link2 v-else :size="16"/>与房主同时建立连接</button>
      <div v-else class="local-address"><span>Minecraft 多人游戏地址</span><strong>{{ room.localAddress }}</strong><button class="button secondary" @click="copy(room.localAddress, '地址')"><Copy :size="16"/>复制地址</button></div>
    </template>
    <button class="button danger close-room" :disabled="busy" @click="closeRoom"><Unplug :size="16"/>关闭联机</button>
  </section>
</template>
