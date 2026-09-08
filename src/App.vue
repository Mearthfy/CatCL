<script setup lang="ts">
import { onMounted } from 'vue'
import { Activity, ArrowDownToLine, ArrowRight, Box, Check, ChevronRight, Coffee, Download, Folder, FolderOpen, House, LoaderCircle, Package, Radio, RefreshCw, Search, Settings2, ShieldCheck, Terminal, Trash2, X } from '@lucide/vue'
import WindowControls from './components/WindowControls.vue'
import WorldManager from './components/WorldManager.vue'
import AccountManager from './components/AccountManager.vue'
import ResourceBrowser from './components/ResourceBrowser.vue'
import Multiplayer from './components/Multiplayer.vue'
import catclLogo from './assets/catcl-logo.jpg'
import { useLauncherStore } from './stores/launcher'
import type { Page } from './types'

const store = useLauncherStore()
const navigation = [
  { id: 'home' as Page, label: '开始游戏', icon: House },
  { id: 'versions' as Page, label: '版本管理', icon: Box },
  { id: 'resources' as Page, label: '资源广场', icon: Package },
  { id: 'worlds' as Page, label: '世界管理', icon: Folder },
  { id: 'multiplayer' as Page, label: '联机', icon: Radio },
  { id: 'downloads' as Page, label: '下载任务', icon: Download },
  { id: 'settings' as Page, label: '启动器设置', icon: Settings2 },
]
const titles: Record<Page, string> = { home: '你的冒险基地', versions: '发现你的下一个世界', resources: '探索 Minecraft 社区生态', worlds: '收藏每一段冒险', multiplayer: '和朋友直接连接', downloads: '准备冒险所需的一切', settings: '让一切顺手起来' }
function changeInstance(event: Event) { store.selectInstance((event.target as HTMLSelectElement).value) }
function changeLoader(event: Event) { const value = (event.target as HTMLSelectElement).value; if (value === 'fabric' || value === 'forge' || value === 'neoforge') store.prepareLoader(value) }
async function requestDeleteInstance(name: string) {
  if (!window.confirm(`确定删除实例“${name}”吗？实例文件夹、模组、配置和世界存档都会被永久删除。`)) return
  await store.deleteInstance(name)
}
onMounted(() => store.initialize())
</script>

<template>
  <div class="launcher">
    <aside class="sidebar">
      <div class="profile"><img class="catcl-logo" :src="catclLogo" alt="CatCL · CCL"/><span class="profile-caption">{{ store.settings.offlineName || 'Player' }} · 离线模式</span></div>
      <nav aria-label="主导航"><button v-for="item in navigation" :key="item.id" :class="['nav-item', { active: store.page === item.id }]" :aria-current="store.page === item.id ? 'page' : undefined" @click="store.page = item.id"><component :is="item.icon" :size="19"/><span>{{ item.label }}</span><i v-if="item.id === 'downloads' && (store.installing || store.resourceDownloading)" class="activity-dot"/></button></nav>
    </aside>

    <main>
      <header class="topbar" data-tauri-drag-region><div class="heading-copy" data-tauri-drag-region><div class="eyebrow" data-tauri-drag-region>MINECRAFT · JAVA EDITION</div><h1 data-tauri-drag-region>{{ titles[store.page] }}</h1></div><WindowControls @error="store.error = $event"/></header>
      <div class="page-content" :class="{ 'home-content': store.page === 'home' }">
      <span class="platform" :title="store.desktop ? 'Rust + Tauri' : '运行 npm run desktop 使用桌面功能'">{{ store.desktop ? 'Windows 桌面版' : '浏览器预览' }}</span>
      <div v-if="store.error" class="banner error" role="alert"><span>{{ store.error }}</span><button class="icon-button" aria-label="关闭错误提示" @click="store.error = ''"><X :size="16"/></button></div>
      <div v-if="store.notice" class="banner success" role="status"><Check :size="17"/><span>{{ store.notice }}</span><button class="icon-button" aria-label="关闭提示" @click="store.notice = ''"><X :size="16"/></button></div>
      <label class="instance-selector"><span><Box :size="17"/><span>当前实例</span></span><select :value="store.instanceSelection" :disabled="store.installing || store.gameRunning" aria-label="选择实例" @change="changeInstance"><option value="__empty__">空实例 · 浏览全部资源版本</option><option v-for="instance in store.settings.instances" :key="instance.name" :value="instance.name">{{ instance.name }} · Minecraft {{ instance.version }}</option></select><small :title="store.selectedInstance?.path">{{ store.emptyInstanceSelected ? '不绑定版本、加载器或目录' : store.selectedInstance?.path }}</small></label>

      <template v-if="store.page === 'home'">
        <section class="hero">
          <div class="hero-content"><span class="pill"><span class="activity-dot"/> 无限世界，随你探索</span><h2>把日常留在身后。<br/>从一个方块开始。</h2><p>山野、洞穴，还有未完成的建筑。<br/>你的下一个故事，就在这里。</p><button class="button light" @click="store.page = 'versions'">探索游戏版本 <ArrowRight :size="17"/></button></div>
        </section>
        <div class="section-heading"><h2>准备出发</h2><span>VANILLA EXPERIENCE</span></div>
        <section class="launch-card">
          <div class="block-icon"><Box :size="30"/></div><div class="launch-info"><h3>{{ store.selectedInstance?.name || '选择一个游戏实例' }} <span class="tag">{{ store.selectedInstance?.version || '原版' }}</span></h3><p>{{ store.selectedInstance ? store.selectedInstance.path : '每个实例拥有独立的游戏文件夹与世界存档' }}</p></div>
          <template v-if="store.selectedInstance"><button class="button secondary" :disabled="!store.desktop || store.installing || store.gameRunning" title="启用 Java Flight Recorder 并保存启动诊断文件" @click="store.launch(true)"><Activity :size="17"/>分析启动速度</button><button class="button primary" :disabled="!store.desktop || store.installing || store.gameRunning" @click="store.launch(false)"><ArrowRight :size="18"/>{{ store.gameRunning ? '游戏运行中' : '离线启动' }}</button></template>
          <button v-else class="button primary" @click="store.page = 'versions'"><ArrowDownToLine :size="18"/>创建实例</button>
        </section>
        <div class="quick-grid"><button class="quick-card" @click="store.page = 'settings'"><Coffee :size="23"/><div><strong>运行环境</strong><span>{{ store.java ? `Java ${store.java.major}` : '配置并检测 Java' }}</span></div><ChevronRight :size="17"/></button><button class="quick-card" @click="store.page = 'settings'"><Settings2 :size="23"/><div><strong>内存分配</strong><span>{{ (store.settings.memoryMb / 1024).toFixed(1) }} GB · 可自由调整</span></div><ChevronRight :size="17"/></button></div>
        <p class="footnote"><ShieldCheck :size="15"/> 离线身份会生成稳定 UUID；需要正版验证的服务器仍需等待微软账号接入。</p>
      </template>

      <template v-else-if="store.page === 'versions'">
        <section class="panel instance-folder-panel">
          <div class="section-heading"><h2><Folder :size="20"/>实例文件夹</h2><span>自动寻找版本与模组加载器</span></div>
          <p class="field-help">选择一个实例文件夹，或选择包含多个独立实例子文件夹的目录。CCL 会读取 versions 中的 JSON 自动建立实例列表。</p>
          <div class="input-action"><input :value="store.settings.gameDir" readonly title="实例文件夹"/><button class="button primary" type="button" :disabled="!store.desktop || store.scanningInstances || store.gameRunning" @click="store.scanInstanceFolder(true)"><FolderOpen :size="16"/>{{ store.scanningInstances ? '扫描中…' : '选择文件夹' }}</button><button class="button secondary" type="button" :disabled="!store.desktop || store.scanningInstances || !store.settings.gameDir" @click="store.scanInstanceFolder(false)"><RefreshCw :size="16" :class="{ spinning: store.scanningInstances }"/>重新扫描</button></div>
        </section>
        <section v-if="store.settings.instances.length" class="panel instance-panel"><div class="section-heading"><h2>我的实例 <span class="count">{{ store.settings.instances.length }}</span></h2><span>名称唯一 · 文件夹互相独立</span></div><div class="instance-list"><div v-for="instance in store.settings.instances" :key="instance.name" :class="['instance-row', { selected: store.settings.selectedInstance === instance.name }]"><button class="instance-select" @click="store.selectInstance(instance.name)"><div class="version-symbol"><Box :size="20"/></div><div><strong>{{ instance.name }}</strong><span>Minecraft {{ instance.version }} · {{ instance.loader || '原版' }} · {{ instance.path }}</span></div><Check v-if="store.settings.selectedInstance === instance.name" :size="19" class="selected-check"/></button><button class="instance-delete" :aria-label="`删除实例 ${instance.name}`" :disabled="store.installing || store.resourceDownloading || store.gameRunning" @click="requestDeleteInstance(instance.name)"><Trash2 :size="16"/></button></div></div></section>
        <div class="loader-tabs" role="tablist" aria-label="版本类型"><button :class="{ active: store.versionChannel === 'release' }" role="tab" :aria-selected="store.versionChannel === 'release'" @click="store.versionChannel = 'release'">正式版</button><button :class="{ active: store.versionChannel === 'snapshot' }" role="tab" :aria-selected="store.versionChannel === 'snapshot'" @click="store.versionChannel = 'snapshot'">快照</button></div>
        <section v-if="store.selectedInstance" class="panel loader-install"><div class="section-heading"><h2>模组加载器管理</h2><span>目标：{{ store.selectedInstance.name }} · Minecraft {{ store.selectedInstance.version }}</span></div><div v-if="store.selectedInstance.loader" class="banner"><ShieldCheck :size="17"/><span>已安装 {{ store.selectedInstance.loader }}。为避免实例损坏，不能重复安装或叠加其他加载器。</span></div><div class="loader-picker"><label>加载器<select :value="store.loaderKind || ''" :disabled="!store.desktop || store.installingLoader || !!store.selectedInstance.loader" aria-label="选择模组加载器" @change="changeLoader"><option disabled value="">选择 Fabric / Forge / NeoForge</option><option value="fabric">Fabric</option><option value="forge">Forge</option><option value="neoforge">NeoForge</option></select></label><label>加载器版本<select v-model="store.selectedLoaderVersion" :disabled="!store.loaderKind || store.loadingLoaderVersions || store.installingLoader || !!store.selectedInstance.loader" aria-label="选择加载器版本"><option v-if="store.loadingLoaderVersions" value="">正在读取兼容版本…</option><option v-for="version in store.loaderVersions" :key="version" :value="version">{{ version }}</option></select></label><button class="button primary" :disabled="!store.desktop || !store.selectedLoaderVersion || store.installingLoader || !!store.selectedInstance.loader" @click="store.installLoader"><Download :size="16"/>{{ store.installingLoader ? '安装中' : '安装所选版本' }}</button></div><p class="field-help">版本列表只显示与当前 Minecraft 实例兼容的官方构建；每个实例只能安装一种加载器。</p></section>
        <div class="toolbar"><label class="search-field"><Search :size="18"/><input v-model="store.search" placeholder="搜索游戏版本…" aria-label="搜索游戏版本"/></label><button class="button secondary" :disabled="!store.desktop || store.inspectingModpack" @click="store.inspectModpack"><FolderOpen :size="16"/>{{ store.inspectingModpack ? '识别中' : '识别整合包' }}</button><button class="button secondary" :disabled="!store.desktop || store.loading" @click="store.refresh"><RefreshCw :size="16" :class="{ spinning: store.loading }"/>刷新</button></div>
        <section v-if="store.modpack" class="panel modpack-result"><div class="section-heading"><h2>{{ store.modpack.name || '未命名整合包' }} <span class="tag">{{ store.modpack.format }}</span></h2><span>{{ store.modpack.fileCount }} 个文件</span></div><div class="modpack-details"><span>整合包版本<strong>{{ store.modpack.version || '未提供' }}</strong></span><span>Minecraft<strong>{{ store.modpack.minecraftVersion || '未识别' }}</strong></span><span>加载器<strong>{{ store.modpack.loaders.join('、') || '未识别' }}</strong></span></div><p class="field-help" :title="store.modpackPath">{{ store.modpackPath }}</p><p v-for="warning in store.modpack.warnings" :key="warning" class="banner error">{{ warning }}</p><div class="action-bar"><span>目标：<strong>{{ store.selectedInstance?.name || '请先选择实例' }}</strong></span><button class="button primary" :disabled="!store.desktop || !store.selectedInstance || store.applyingModpack" @click="store.applyModpack"><Download :size="16"/>{{ store.applyingModpack ? '应用中' : '应用到当前实例' }}</button></div></section>
        <section v-if="store.modpack && store.modpack.minecraftVersion" class="panel modpack-import">
          <div class="section-heading"><h2>导入为新实例</h2><span>自动安装 Minecraft {{ store.modpack.minecraftVersion }}</span></div>
          <div class="instance-fields">
            <label>实例名称<input v-model.trim="store.modpackInstanceName" maxlength="40" placeholder="整合包实例名称"></label>
            <label>独立文件夹<div class="input-action"><input v-model.trim="store.modpackInstancePath" placeholder="选择独立实例文件夹"><button class="button secondary" type="button" :disabled="!store.desktop || store.installing" @click="store.browseModpackInstancePath"><FolderOpen :size="16"/>浏览</button></div></label>
          </div>
          <div class="action-bar"><span>将自动下载游戏版本并应用整合包。</span><button class="button primary" :disabled="!store.desktop || store.installing || store.resourceDownloading || !store.modpackInstanceName || !store.modpackInstancePath" @click="store.importModpackAsInstance"><Download :size="16"/>自动导入</button></div>
        </section>
        <div class="section-heading"><h2>{{ store.versionChannel === 'release' ? '原版正式版' : '原版快照' }} <span class="count">{{ store.filteredVersions.length }}</span></h2><span>各加载器独立管理</span></div>
        <div v-if="store.loading" class="empty-state"><LoaderCircle class="spinning" :size="30"/><h3>正在读取官方版本列表</h3><p>首次加载需要连接 Mojang 服务。</p></div>
        <div v-else-if="!store.filteredVersions.length" class="empty-state"><Box :size="36"/><h3>{{ store.versions.length ? '没有匹配的版本' : '还没有版本信息' }}</h3><p>{{ store.desktop ? '尝试刷新列表，或检查网络连接。' : '在桌面版中加载真实官方版本列表。' }}</p></div>
        <div v-else class="version-list"><button v-for="version in store.filteredVersions" :key="version.id" :disabled="store.installing" :class="['version-row', { selected: store.settings.selectedVersion === version.id }]" @click="store.chooseVersion(version.id)"><div class="version-symbol"><Box :size="22"/></div><div><strong>{{ version.id }}</strong><span>{{ version.type === 'release' ? '正式版' : '快照' }} · {{ new Date(version.releaseTime).toLocaleDateString('zh-CN') }}</span></div><Check v-if="store.settings.selectedVersion === version.id" :size="19" class="selected-check"/></button></div>
        <section v-if="store.settings.selectedVersion" class="panel create-instance"><div class="section-heading"><h2>创建独立实例</h2><span>Minecraft {{ store.settings.selectedVersion }}</span></div><div class="instance-fields"><label>实例名称<input v-model.trim="store.instanceName" maxlength="40" placeholder="例如：我的生存服"/></label><label>独立文件夹<div class="input-action"><input v-model.trim="store.instancePath" placeholder="例如 D:\Minecraft\我的生存服"/><button class="button secondary" type="button" :disabled="!store.desktop" @click="store.browseInstancePath"><FolderOpen :size="16"/>浏览</button></div></label></div><div class="action-bar"><span>同一版本可创建多个实例，但名称和文件夹不能重复。</span><button class="button primary" :disabled="!store.desktop || store.installing || !store.instanceName || !store.instancePath" @click="store.install"><Download :size="17"/>{{ store.installing ? '正在下载' : '下载为新实例' }}</button></div></section>
      </template>

      <ResourceBrowser v-else-if="store.page === 'resources'"/>
      <WorldManager v-else-if="store.page === 'worlds'"/>
      <Multiplayer v-else-if="store.page === 'multiplayer'" :instance="store.selectedInstance"/>
      <template v-else-if="store.page === 'downloads'">
        <section class="panel download-panel"><div class="section-heading"><h2>下载任务</h2><span>{{ store.installing || store.resourceDownloading ? '正在处理' : '空闲' }}</span></div><template v-if="store.downloadTask"><div class="download-heading"><div class="block-icon"><Package :size="26"/></div><div><h3>{{ store.downloadTask.title }}</h3><p>{{ store.downloadTask.message }}</p></div><strong>{{ store.downloadTask.percent }}%</strong></div><progress :value="store.downloadTask.percent" max="100" aria-label="资源下载进度"/><div class="download-meta"><span>{{ store.downloadTask.detail || (store.downloadTask.status === 'complete' ? '任务已完成' : store.downloadTask.status === 'error' ? '任务失败' : '正在连接下载源…') }}</span><span class="tag">{{ store.downloadTask.status === 'active' ? '下载中' : store.downloadTask.status === 'complete' ? '已完成' : '失败' }}</span></div></template><template v-else-if="store.progress"><div class="download-heading"><div class="block-icon"><Download :size="26"/></div><div><h3>Minecraft {{ store.progress.version }}</h3><p>{{ store.progress.message }}</p></div><strong>{{ store.percent }}%</strong></div><progress :value="store.percent" max="100" aria-label="下载进度"/><div class="download-meta"><span>{{ store.progress.completed }} / {{ store.progress.total }} 个文件 · SHA-1 校验</span><button v-if="store.installing" class="button secondary" @click="store.cancel">取消下载</button></div></template><div v-else class="empty-state"><Download :size="36"/><h3>没有进行中的下载</h3><p>下载游戏版本或社区资源后，进度会显示在这里。</p><button class="button secondary" @click="store.page = 'versions'">选择版本 <ArrowRight :size="16"/></button></div></section>
        <section class="panel log-panel"><div class="section-heading"><h2><Terminal :size="17"/>活动与游戏日志</h2><span v-if="store.gameRunning" class="running-label"><i class="activity-dot"/> Minecraft 运行中</span><button class="text-button" @click="store.logs = []">清空</button></div><div class="logs" role="log" aria-live="polite"><p v-if="!store.logs.length" class="muted">暂无日志。</p><div v-for="(entry, index) in store.logs" :key="index" :class="['log-row', entry.level]"><time>{{ entry.time }}</time><span>{{ entry.message }}</span></div></div></section>
      </template>

      <template v-else>
        <section class="panel settings-panel"><div class="section-heading"><h2><Coffee :size="20"/>Java 运行环境</h2><span>TEMURIN RUNTIME</span></div><label for="java-path">Java 可执行文件</label><p class="field-help">CatCL 可根据当前选中的实例自动安装匹配的 Windows x64 Java，也可以填写已有的 java.exe。</p><div class="input-action"><input id="java-path" v-model="store.settings.javaPath" :disabled="store.installingJava || store.installing" placeholder="C:\Program Files\Java\jdk-21\bin\java.exe"/><button class="button secondary" type="button" :disabled="!store.desktop || store.installingJava" @click="store.browseJava"><FolderOpen :size="16"/>浏览</button><button class="button secondary" :disabled="!store.desktop || store.checkingJava || store.installingJava" @click="store.checkJava"><RefreshCw v-if="store.checkingJava" class="spinning" :size="16"/>检测</button><button class="button primary" :disabled="!store.desktop || !store.selectedInstance || store.installingJava" @click="store.installJava"><Download :size="16"/>{{ store.installingJava ? '安装中' : '自动安装' }}</button></div><div v-if="store.javaProgress" class="java-download"><div><span>{{ store.javaProgress.message }}</span><strong>{{ store.javaProgress.total ? Math.floor(store.javaProgress.downloaded / store.javaProgress.total * 100) : 0 }}%</strong></div><progress :value="store.javaProgress.downloaded" :max="store.javaProgress.total || 1" aria-label="Java 下载进度"/></div><div v-if="store.java" class="java-result"><Check :size="16"/> Java {{ store.java.major }} · {{ store.java.is64Bit ? '64 位' : '请确认安装 64 位 Java' }}<pre>{{ store.java.version }}</pre></div></section>
        <section class="panel settings-panel"><div class="section-heading"><h2><Settings2 :size="20"/>游戏设置</h2></div><label for="offline-name">离线玩家名</label><p class="field-help">使用 3–16 位英文字母、数字或下划线；相同名字会得到相同 UUID。</p><input id="offline-name" v-model.trim="store.settings.offlineName" :disabled="store.installing || store.gameRunning" minlength="3" maxlength="16" pattern="[A-Za-z0-9_]{3,16}" placeholder="Player"/><label for="memory">最大内存 <span class="memory-value">{{ (store.settings.memoryMb / 1024).toFixed(1) }} GB</span></label><input id="memory" v-model.number="store.settings.memoryMb" :disabled="store.installing" type="range" min="1024" max="16384" step="512"/><div class="range-labels"><span>1 GB</span><span>16 GB</span></div><label for="game-dir" class="directory-label"><Folder :size="17"/>新实例默认父目录</label><p class="field-help">这里只用于生成新实例的建议路径；每个实例都可在下载前改成电脑上的任意独立文件夹。</p><div class="input-action"><input id="game-dir" v-model="store.settings.gameDir" :disabled="store.installing" placeholder="例如 D:\Minecraft"/><button class="button secondary" type="button" :disabled="!store.desktop || store.installing" @click="store.browseDefaultPath"><FolderOpen :size="16"/>浏览</button></div></section>
        <div class="action-bar"><span>设置保存在本机应用数据目录</span><button class="button primary" :disabled="!store.desktop || store.saving || store.installing" @click="store.save"><Check :size="17"/>{{ store.saving ? '正在保存…' : '保存设置' }}</button></div>
      </template>
      <footer v-if="store.page !== 'home'"><span>CatCL (CCL) · 开发预览 v0.1.0</span><span>非 Minecraft 官方产品</span></footer>
      <section v-if="store.page === 'settings'" class="panel settings-panel"><div class="section-heading"><h2><Activity :size="20"/>实验性启动加速</h2><span>STARTUP HOTSET</span></div><label class="hotset-option"><input v-model="store.settings.experimentalHotset" type="checkbox" :disabled="store.gameRunning"/><span><strong>启用 Startup HotSet</strong><small>后台低优先级预读，最多 256MB；检测到启动变慢超过 5% 时自动停用。</small></span></label></section>
      <AccountManager v-if="store.page === 'settings'"/>
      </div>
    </main>
  </div>
</template>
