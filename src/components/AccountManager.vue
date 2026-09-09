<script setup lang="ts">
import { Check, Copy, ExternalLink, LogIn, LogOut, ShieldCheck, X } from '@lucide/vue'
import { useLauncherStore } from '../stores/launcher'
const store = useLauncherStore()
async function copyCode() {
  if (store.microsoftChallenge) await navigator.clipboard.writeText(store.microsoftChallenge.userCode)
}
</script>

<template>
  <section class="panel settings-panel account-panel">
    <div class="section-heading"><h2><ShieldCheck :size="20"/>游戏账户</h2><span>{{ store.account?.playerName || '未登录' }}</span></div>
    <label>启动方式</label>
    <select v-model="store.settings.accountMode" :disabled="store.gameRunning || store.loggingIn">
      <option value="offline">离线模式（默认无皮肤）</option><option value="littleskin">LittleSkin 外置登录</option><option value="microsoft">Microsoft 正版</option>
    </select>
    <p v-if="store.settings.accountMode === 'offline'" class="field-help">离线模式使用本地用户名和稳定 UUID，不加载启动器皮肤。</p>
    <template v-else-if="store.settings.accountMode === 'littleskin'">
      <label class="account-field"><span>LittleSkin 邮箱或账户</span><input v-model.trim="store.accountUsername" autocomplete="username" :disabled="store.loggingIn"/></label>
      <label class="account-field"><span>密码</span><input v-model="store.accountPassword" type="password" autocomplete="current-password" :disabled="store.loggingIn" @keyup.enter="store.loginLittleSkin"/></label>
      <button class="button primary" :disabled="store.loggingIn || !store.accountUsername || !store.accountPassword" @click="store.loginLittleSkin"><LogIn :size="16"/>{{ store.loggingIn ? '登录中…' : '登录 LittleSkin' }}</button>
    </template>
    <template v-else>
      <p class="field-help">CCL 已配置专用 Microsoft 应用。点击后会显示可复制的验证码，并询问是否打开授权网页。</p>
      <button class="button primary" :disabled="store.loggingIn" @click="store.loginMicrosoft"><LogIn :size="16"/>{{ store.loggingIn ? '等待授权…' : 'Microsoft 设备登录' }}</button>
    </template>
    <div v-if="store.account" class="java-result"><Check :size="16"/>{{ store.account.mode === 'microsoft' ? 'Microsoft 正版' : 'LittleSkin' }} · {{ store.account.playerName }} · {{ store.account.uuid }} <button class="text-button" @click="store.logoutAccount"><LogOut :size="14"/>退出</button></div>
  </section>
  <div v-if="store.microsoftChallenge" class="auth-overlay" role="dialog" aria-modal="true" aria-label="Microsoft 设备登录"><section class="auth-dialog"><button class="auth-close" aria-label="关闭验证码窗口" @click="store.microsoftChallenge = null"><X :size="17"/></button><ShieldCheck :size="34"/><h2>Microsoft 正版授权</h2><p>在 Microsoft 页面输入验证码。授权通过后，CCL 会自动切换到正版账户。</p><button class="device-code" title="复制验证码" @click="copyCode">{{ store.microsoftChallenge.userCode }} <Copy :size="17"/></button><div class="dialog-actions"><button class="button secondary" @click="copyCode"><Copy :size="16"/>复制验证码</button><button class="button primary" @click="store.openMicrosoftPage"><ExternalLink :size="16"/>打开授权页</button></div><small>{{ store.microsoftChallenge.verificationUri }}</small></section></div>
</template>
