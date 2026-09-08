<script setup lang="ts">
import { computed, ref } from 'vue'
import { ArrowLeft, CalendarDays, Download, Layers3, Package, RefreshCw, Search, UserRound } from '@lucide/vue'
import { backend } from '../services/backend'
import { useLauncherStore } from '../stores/launcher'
import type { ResourceProject, ResourceVersionInfo } from '../types'

const launcher = useLauncherStore()
const query = ref('')
const source = ref<'modrinth' | 'mcmod' | 'curseforge'>('modrinth')
const kind = ref<'mod' | 'modpack' | 'shader' | 'resourcepack' | 'datapack'>('mod')
const projects = ref<ResourceProject[]>([])
const loading = ref(false)
const installing = ref('')
const error = ref('')
const notice = ref('')
const detailProject = ref<ResourceProject | null>(null)
const versions = ref<ResourceVersionInfo[]>([])
const loadingVersions = ref(false)
const loader = computed(() => launcher.selectedInstance?.loader?.split(' ')[0].toLowerCase() ?? '')
const kinds = [
  { id: 'mod', label: '模组' }, { id: 'modpack', label: '整合包' }, { id: 'shader', label: '光影包' },
  { id: 'resourcepack', label: '资源包' }, { id: 'datapack', label: '数据包' },
] as const
const message = (value: unknown) => value instanceof Error ? value.message : String(value)

function selectKind(value: typeof kind.value) {
  kind.value = value
  projects.value = []
  error.value = ''
  notice.value = ''
}

async function search() {
  if (source.value === 'mcmod') {
    error.value = ''
    notice.value = ''
    try {
      await backend.openResourceSite('mcmod', query.value)
      notice.value = '已在浏览器打开 MC百科。可查看中文资料、依赖关系和其提供的授权下载来源。'
    } catch (e) { error.value = message(e) }
    return
  }
  if (source.value === 'curseforge') {
    error.value = ''
    notice.value = ''
    try {
      await backend.openResourceSite('curseforge', query.value)
      notice.value = '已在浏览器打开 CurseForge 搜索。直接集成下载需要 CurseForge 授权的 API 密钥。'
    } catch (e) { error.value = message(e) }
    return
  }
  const instance = launcher.selectedInstance
  loading.value = true
  error.value = ''
  notice.value = ''
  try {
    projects.value = await backend.searchResources(query.value.trim(), kind.value,
      launcher.emptyInstanceSelected ? '' : instance?.version ?? '',
      launcher.emptyInstanceSelected ? '' : loader.value)
  } catch (e) { error.value = message(e) }
  finally { loading.value = false }
}

async function install(project: ResourceProject) {
  const instance = launcher.selectedInstance
  if (launcher.emptyInstanceSelected || !instance || installing.value || launcher.resourceDownloading) return
  if (project.projectType === 'mod' && !loader.value) {
    error.value = '可以浏览模组，但安装前需要先给实例选择 Fabric、Forge 或 NeoForge。'
    return
  }
  installing.value = project.id
  error.value = ''
  notice.value = ''
  let unlisten: (() => void) | undefined
  try {
    launcher.startDownloadTask(project.title, '正在准备资源下载…')
    unlisten = await backend.onResourceProgress(progress => {
      if (progress.id !== project.id) return
      launcher.updateDownloadTask(
        progress.total ? Math.round(progress.downloaded / progress.total * 100) : 0,
        progress.message,
        `${formatBytes(progress.downloaded)} / ${progress.total ? formatBytes(progress.total) : '未知大小'}`,
      )
    })
    const result = await backend.installResource(project.id, project.projectType, instance.path, instance.version, loader.value)
    if (result.projectType === 'modpack') {
      const info = await backend.inspectModpack(result.path)
      const applied = await backend.applyModpack(result.path, instance.path, instance.version)
      notice.value = `${info.name || project.title} 已应用：${applied.appliedFiles + applied.downloadedFiles} 个文件`
    } else notice.value = `${project.title} 已下载到当前实例`
    launcher.finishDownloadTask(notice.value)
  } catch (e) { error.value = message(e); launcher.failDownloadTask(e) }
  finally { unlisten?.(); installing.value = '' }
}

async function downloadModpack(project: ResourceProject, version: ResourceVersionInfo) {
  if (installing.value || launcher.resourceDownloading) return
  error.value = ''
  notice.value = ''
  let unlistenResource: (() => void) | undefined
  let unlistenGame: (() => void) | undefined
  let unlistenPack: (() => void) | undefined
  try {
    const destination = await backend.pickDirectory()
    if (!destination) return
    installing.value = version.id
    launcher.startDownloadTask(`${project.title} · ${version.versionNumber}`, '正在准备整合包下载…')
    unlistenResource = await backend.onResourceProgress(progress => {
      if (progress.id !== version.id) return
      launcher.updateDownloadTask(
        progress.total ? Math.round(progress.downloaded / progress.total * 100) : 0,
        progress.message,
        `${formatBytes(progress.downloaded)} / ${progress.total ? formatBytes(progress.total) : '未知大小'}`,
      )
    })
    unlistenGame = await backend.onProgress(progress => {
      launcher.updateDownloadTask(
        progress.total ? Math.round(progress.completed / progress.total * 100) : 0,
        progress.message,
        progress.total ? `${progress.completed} / ${progress.total} 个游戏文件` : '',
      )
    })
    unlistenPack = await backend.onModpackProgress(progress => {
      launcher.updateDownloadTask(
        progress.total ? Math.round(progress.completed / progress.total * 100) : 0,
        progress.message,
        progress.total ? `${progress.completed} / ${progress.total} 个应用步骤` : '',
      )
    })
    const result = await backend.downloadResource(project.id, version.id, project.projectType, destination)
    const pack = await backend.inspectModpack(result.path)
    if (!pack.minecraftVersion) throw new Error('整合包没有声明 Minecraft 版本，无法自动创建实例')

    const baseName = (pack.name || project.title)
      .replace(/[<>:"/\\|?*\u0000-\u001F]/g, ' ')
      .replace(/[. ]+$/g, '')
      .trim()
      .slice(0, 36) || '整合包实例'
    let instanceName = baseName
    let suffix = 1
    while (launcher.settings.instances.some(item => item.name.toLocaleLowerCase() === instanceName.toLocaleLowerCase())) {
      instanceName = `${baseName.slice(0, 32)} (${++suffix})`
    }
    const separator = destination.endsWith('\\') || destination.endsWith('/') ? '' : '\\'
    const instancePath = `${destination}${separator}${instanceName}`
    const normalizedPath = instancePath.replaceAll('/', '\\').replace(/[\\]+$/, '').toLocaleLowerCase()
    if (launcher.settings.instances.some(item => {
      const existing = item.path.replaceAll('/', '\\').replace(/[\\]+$/, '').toLocaleLowerCase()
      return existing === normalizedPath || existing.startsWith(`${normalizedPath}\\`) || normalizedPath.startsWith(`${existing}\\`)
    })) throw new Error('自动生成的实例目录与已有实例重叠，请选择其他文件夹')

    notice.value = `正在下载 Minecraft ${pack.minecraftVersion}…`
    await backend.install(pack.minecraftVersion, instancePath)
    const created: typeof launcher.settings.instances[number] = { name: instanceName, version: pack.minecraftVersion, path: instancePath }
    const loaderSpec = pack.loaders.find(value => /^(fabric-loader|fabric|forge|neoforge)\s+/i.test(value))
    if (loaderSpec) {
      const separator = loaderSpec.indexOf(' ')
      const rawLoader = loaderSpec.slice(0, separator).toLowerCase()
      const loaderName = rawLoader === 'fabric-loader' ? 'fabric' : rawLoader
      const loaderVersion = loaderSpec.slice(separator + 1).trim()
      if (loaderName === 'fabric' || loaderName === 'forge' || loaderName === 'neoforge') {
        launcher.updateDownloadTask(0, `正在安装 ${loaderName} ${loaderVersion}…`)
        const installedLoader = await backend.installLoader(instancePath, pack.minecraftVersion, loaderName, loaderVersion, launcher.settings.javaPath)
        created.loader = `${installedLoader.loader} ${installedLoader.loaderVersion}`
        created.launchVersion = installedLoader.launchVersion
      }
    }
    launcher.settings.instances.push(created)
    launcher.settings.selectedInstance = instanceName
    launcher.settings.selectedVersion = pack.minecraftVersion
    launcher.emptyInstanceSelected = false
    await backend.saveSettings({ ...launcher.settings })

    const applied = await backend.applyModpack(result.path, instancePath, pack.minecraftVersion)
    notice.value = `${instanceName} 已创建：Minecraft ${pack.minecraftVersion}，应用 ${applied.appliedFiles + applied.downloadedFiles} 个整合包文件`
    launcher.finishDownloadTask('整合包实例安装完成', notice.value)
  } catch (e) { error.value = message(e); launcher.failDownloadTask(e) }
  finally { unlistenResource?.(); unlistenGame?.(); unlistenPack?.(); installing.value = '' }
}

function formatBytes(value: number) {
  if (value < 1024) return `${value} B`
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`
  return `${(value / 1024 / 1024).toFixed(1)} MB`
}

async function openProject(project: ResourceProject) {
  detailProject.value = project
  versions.value = []
  loadingVersions.value = true
  error.value = ''
  notice.value = ''
  try {
    versions.value = await backend.resourceVersions(project.id,
      launcher.emptyInstanceSelected ? '' : launcher.selectedInstance?.version ?? '',
      launcher.emptyInstanceSelected ? '' : loader.value)
  } catch (e) { error.value = message(e) }
  finally { loadingVersions.value = false }
}

function closeProject() {
  detailProject.value = null
  versions.value = []
  error.value = ''
  notice.value = ''
}
function hideBrokenImage(event: Event) { (event.currentTarget as HTMLImageElement).style.display = 'none' }
const dependencyLabel = (type: string) => ({ required: '必需', optional: '可选', incompatible: '不兼容', embedded: '内置' }[type] ?? type)
</script>

<template>
  <div class="resource-browser">
    <template v-if="detailProject">
      <button class="resource-back" type="button" @click="closeProject"><ArrowLeft :size="17"/>返回资源列表</button>
      <section class="resource-detail-hero">
        <div class="resource-cover resource-cover-large">
          <span>{{ detailProject.title.slice(0, 1).toUpperCase() }}</span>
          <img v-if="detailProject.iconUrl" :src="detailProject.iconUrl" :alt="`${detailProject.title} 封面`" @error="hideBrokenImage">
        </div>
        <div class="resource-detail-copy">
          <span class="resource-kind">{{ kinds.find(item => item.id === detailProject?.projectType)?.label || detailProject.projectType }}</span>
          <h2>{{ detailProject.title }}</h2>
          <p>{{ detailProject.description || '这个组件暂时没有简介。' }}</p>
          <div class="resource-meta">
            <span><UserRound :size="14"/>{{ detailProject.author }}</span>
            <span><Download :size="14"/>{{ detailProject.downloads.toLocaleString('zh-CN') }} 次下载</span>
            <span><Layers3 :size="14"/>{{ versions.length }} 个版本</span>
          </div>
        </div>
        <button v-if="!launcher.emptyInstanceSelected" class="button primary resource-detail-install" :disabled="!!installing" @click="install(detailProject)">
          <Download :size="16"/>{{ installing === detailProject.id ? '下载中' : detailProject.projectType === 'modpack' ? '下载并应用' : '安装到当前实例' }}
        </button>
      </section>
      <p v-if="launcher.emptyInstanceSelected" class="banner"><span>{{ detailProject.projectType === 'modpack' ? '请在下方选择整合包版本。CatCL 将自动创建独立实例、下载对应 Minecraft 版本并应用整合包。' : '当前为空实例浏览模式。选择一个已下载实例后即可安装这个组件。' }}</span></p>
      <p v-if="error" class="banner error" role="alert"><span>{{ error }}</span></p>
      <p v-if="notice" class="banner success" role="status"><span>{{ notice }}</span></p>
      <section class="resource-detail-versions">
        <div class="section-heading"><h2>全部版本 <span class="count">{{ versions.length }}</span></h2><span>{{ launcher.emptyInstanceSelected ? '显示所有 Minecraft 与加载器版本' : '已按当前实例筛选兼容版本' }}</span></div>
        <div v-if="loadingVersions" class="empty-state compact"><RefreshCw class="spinning" :size="28"/><p>正在读取版本与依赖…</p></div>
        <div v-else-if="!versions.length" class="empty-state compact"><Package :size="28"/><p>没有找到兼容版本</p></div>
        <div v-else class="version-detail-list">
          <article v-for="version in versions" :key="version.id" class="resource-version-card">
            <div class="version-title"><strong>{{ version.versionNumber }}</strong><small>{{ version.name }}</small></div>
            <div class="version-support"><span>Minecraft {{ version.gameVersions.join('、') || '未知' }}</span><span>{{ version.loaders.join('、') || '通用' }}</span></div>
            <time><CalendarDays :size="13"/>{{ new Date(version.datePublished).toLocaleDateString('zh-CN') }}</time>
            <button v-if="launcher.emptyInstanceSelected && detailProject.projectType === 'modpack'" class="button primary version-download" :disabled="!!installing" @click="downloadModpack(detailProject, version)">
              <Download :size="14"/>{{ installing === version.id ? '安装中' : '下载并安装' }}
            </button>
            <div class="dependency-section"><strong>依赖项</strong>
              <div v-if="version.dependencies.length" class="dependency-list"><span v-for="dependency in version.dependencies" :key="`${dependency.projectId}-${dependency.versionId}-${dependency.dependencyType}`" :class="dependency.dependencyType">{{ dependencyLabel(dependency.dependencyType) }} · {{ dependency.title }}</span></div>
              <em v-else>无声明依赖</em>
            </div>
          </article>
        </div>
      </section>
    </template>
    <template v-else>
      <div class="resource-sources">
        <button :class="{ active: source === 'modrinth' }" @click="source = 'modrinth'">Modrinth<small>搜索与安装</small></button>
        <button :class="{ active: source === 'mcmod' }" @click="source = 'mcmod'">MC百科<small>中文资料与依赖</small></button>
        <button :class="{ active: source === 'curseforge' }" @click="source = 'curseforge'">CurseForge<small>更多模组资源</small></button>
      </div>
      <div class="resource-tabs"><button v-for="item in kinds" :key="item.id" :class="{ active: kind === item.id }" @click="selectKind(item.id)">{{ item.label }}</button></div>
      <div class="toolbar"><label class="search-field"><Search :size="17"/><input v-model="query" aria-label="搜索社区资源" placeholder="搜索名称或关键词…" @keyup.enter="search"></label><button class="button primary" :disabled="!launcher.desktop || loading || (!launcher.emptyInstanceSelected && !launcher.selectedInstance)" @click="search"><RefreshCw :size="16" :class="{ spinning: loading }"/>搜索</button></div>
      <p class="field-help">{{ source === 'modrinth' ? 'Modrinth 支持中文名称和常见中文别名' : source === 'mcmod' ? 'MC百科将在系统浏览器中提供中文介绍、关系图和授权下载来源' : 'CurseForge 拥有更多旧版及独占模组，将在系统浏览器中打开搜索结果' }} · {{ launcher.emptyInstanceSelected ? '空实例模式' : `当前实例 ${launcher.selectedInstance?.version || '未选择'}` }}</p>
      <p v-if="error" class="banner error" role="alert"><span>{{ error }}</span></p>
      <p v-if="notice" class="banner success" role="status"><span>{{ notice }}</span></p>
      <div v-if="loading" class="empty-state"><RefreshCw class="spinning" :size="30"/><h3>正在搜索资源</h3></div>
      <div v-else-if="!projects.length" class="empty-state"><Package :size="36"/><h3>还没有搜索结果</h3><p>选择资源类型并搜索，点击结果即可进入组件详情。</p></div>
      <div v-else class="resource-list">
        <article v-for="project in projects" :key="project.id" class="resource-card" role="button" tabindex="0" :aria-label="`查看 ${project.title} 详情`" @click="openProject(project)" @keydown.enter="openProject(project)">
          <div class="resource-cover"><span>{{ project.title.slice(0, 1).toUpperCase() }}</span><img v-if="project.iconUrl" :src="project.iconUrl" :alt="`${project.title} 封面`" loading="lazy" @error="hideBrokenImage"></div>
          <div class="resource-card-copy"><h3>{{ project.title }}</h3><p>{{ project.description }}</p><span>{{ project.author }} · {{ project.downloads.toLocaleString('zh-CN') }} 次下载</span></div>
          <span class="resource-open">查看详情</span>
        </article>
      </div>
    </template>
  </div>
</template>
