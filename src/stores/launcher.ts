import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { backend, desktop } from '../services/backend'
import type { AccountInfo, DownloadTask, GameEvent, GameVersion, InstallProgress, JavaInfo, JavaProgress, LogEntry, MicrosoftChallenge, ModpackInfo, Page, Settings, SkinInfo } from '../types'

export const useLauncherStore = defineStore('launcher', () => {
  const page = ref<Page>('home')
  const settings = ref<Settings>({ javaPath: 'java', offlineName: 'Player', accountMode: 'offline', microsoftClientId: '', memoryMb: 4096, gameDir: '', selectedVersion: null, showSnapshots: false, instances: [], selectedInstance: null, experimentalHotset: false })
  const versions = ref<GameVersion[]>([])
  const installed = ref<string[]>([])
  const instanceName = ref('')
  const instancePath = ref('')
  const modpack = ref<ModpackInfo | null>(null)
  const modpackPath = ref('')
  const modpackInstanceName = ref('')
  const modpackInstancePath = ref('')
  const inspectingModpack = ref(false)
  const scanningInstances = ref(false)
  const installingLoader = ref(false)
  const loadingLoaderVersions = ref(false)
  const loaderKind = ref<'fabric' | 'forge' | 'neoforge' | null>(null)
  const loaderVersions = ref<string[]>([])
  const selectedLoaderVersion = ref('')
  const applyingModpack = ref(false)
  const emptyInstanceSelected = ref(true)
  const java = ref<JavaInfo | null>(null)
  const skin = ref<SkinInfo | null>(null)
  const account = ref<AccountInfo | null>(null)
  const accountUsername = ref('')
  const accountPassword = ref('')
  const loggingIn = ref(false)
  const microsoftChallenge = ref<MicrosoftChallenge | null>(null)
  const logs = ref<LogEntry[]>([])
  const loading = ref(false)
  const saving = ref(false)
  const checkingJava = ref(false)
  const installingJava = ref(false)
  const javaProgress = ref<JavaProgress | null>(null)
  const installing = ref(false)
  const gameRunning = ref(false)
  const initialized = ref(false)
  const error = ref('')
  const notice = ref('')
  const search = ref('')
  const versionChannel = ref<'release' | 'snapshot'>('release')
  const progress = ref<InstallProgress | null>(null)
  const downloadTask = ref<DownloadTask | null>(null)
  const resourceDownloading = computed(() => downloadTask.value?.status === 'active')
  const filteredVersions = computed(() => versions.value.filter(v => v.type === versionChannel.value && v.id.toLowerCase().includes(search.value.toLowerCase())))
  const selected = computed(() => versions.value.find(v => v.id === settings.value.selectedVersion))
  const selectedInstance = computed(() => settings.value.instances.find(instance => instance.name === settings.value.selectedInstance) ?? null)
  const instanceSelection = computed(() => emptyInstanceSelected.value ? '__empty__' : settings.value.selectedInstance ?? '')
  const percent = computed(() => progress.value?.total ? Math.floor(progress.value.completed / progress.value.total * 100) : 0)
  function log(message: string, level: LogEntry['level'] = 'info') {
    logs.value.push({ message, level, time: new Date().toLocaleTimeString('zh-CN', { hour12: false }) })
    if (logs.value.length > 300) logs.value.splice(0, logs.value.length - 300)
  }
  function startDownloadTask(title: string, message: string) {
    downloadTask.value = { title, percent: 0, message, detail: '', status: 'active' }
    page.value = 'downloads'
    log(`开始下载：${title}`)
  }
  function updateDownloadTask(percent: number, message: string, detail = '') {
    if (!downloadTask.value) return
    downloadTask.value = { ...downloadTask.value, percent, message, detail, status: 'active' }
  }
  function finishDownloadTask(message: string, detail = '') {
    if (!downloadTask.value) return
    downloadTask.value = { ...downloadTask.value, percent: 100, message, detail, status: 'complete' }
    log(message)
  }
  function failDownloadTask(reason: unknown) {
    if (!downloadTask.value) return
    const message = reason instanceof Error ? reason.message : String(reason)
    downloadTask.value = { ...downloadTask.value, message, status: 'error' }
    log(message, 'error')
  }
  function fail(reason: unknown) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    log(error.value, 'error')
  }
  function chooseVersion(version: string) {
    settings.value.selectedVersion = version
    let suffix = 1
    let name = version
    while (settings.value.instances.some(instance => instance.name.toLocaleLowerCase() === name.toLocaleLowerCase())) name = `${version} (${++suffix})`
    instanceName.value = name
    const base = settings.value.gameDir.replace(/[\\/]+$/, '')
    instancePath.value = base ? `${base}\\${name}` : ''
  }
  async function browseInstancePath() {
    try {
      const path = await backend.pickDirectory(instancePath.value || settings.value.gameDir)
      if (path) instancePath.value = path
    } catch (e) { fail(e) }
  }
  async function browseDefaultPath() {
    try {
      const path = await backend.pickDirectory(settings.value.gameDir)
      if (path) settings.value.gameDir = path
    } catch (e) { fail(e) }
  }
  async function scanInstanceFolder(selectFolder = false) {
    if (scanningInstances.value) return
    scanningInstances.value = true; error.value = ''; notice.value = ''
    try {
      const path = selectFolder
        ? await backend.pickDirectory(settings.value.gameDir)
        : settings.value.gameDir
      if (!path) return
      const instances = await backend.scanInstanceFolder(path)
      settings.value.gameDir = path
      settings.value.instances = instances
      if (!instances.some(instance => instance.name === settings.value.selectedInstance)) {
        settings.value.selectedInstance = instances[0]?.name ?? null
      }
      emptyInstanceSelected.value = !settings.value.selectedInstance
      installed.value = instances.map(instance => instance.name)
      await backend.saveSettings({ ...settings.value })
      notice.value = instances.length
        ? `已从文件夹找到 ${instances.length} 个实例`
        : '所选文件夹中没有找到可启动的 Minecraft 版本'
      log(notice.value)
    } catch (e) { fail(e) }
    finally { scanningInstances.value = false }
  }
  async function browseJava() {
    try {
      const path = await backend.pickJava(settings.value.javaPath)
      if (path) settings.value.javaPath = path
    } catch (e) { fail(e) }
  }
  async function loadSkin() {
    if (!desktop) return
    try { skin.value = await backend.loadSkin(settings.value.offlineName) } catch (e) { fail(e) }
  }
  async function importSkin() {
    try {
      const path = await backend.pickSkin()
      if (!path) return
      skin.value = await backend.importSkin(settings.value.offlineName, path)
      notice.value = `已为 ${settings.value.offlineName} 导入皮肤`
    } catch (e) { fail(e) }
  }
  async function removeSkin() {
    try { await backend.removeSkin(settings.value.offlineName); skin.value = null; notice.value = '玩家皮肤已移除' } catch (e) { fail(e) }
  }
  async function loginLittleSkin() {
    if (loggingIn.value) return
    loggingIn.value = true; error.value = ''; notice.value = ''
    try {
      account.value = await backend.loginLittleSkin(accountUsername.value, accountPassword.value)
      accountPassword.value = ''
      settings.value.accountMode = 'littleskin'
      await backend.saveSettings({ ...settings.value })
      notice.value = `LittleSkin 已登录：${account.value.playerName}`
    } catch (e) { fail(e) }
    finally { loggingIn.value = false }
  }
  async function logoutAccount() {
    try {
      await backend.logoutAccount(); account.value = null; settings.value.accountMode = 'offline'
      await backend.saveSettings({ ...settings.value }); notice.value = '已切换到无皮肤的离线账户'
    } catch (e) { fail(e) }
  }
  async function loginMicrosoft() {
    if (loggingIn.value) return
    loggingIn.value = true; error.value = ''; notice.value = ''
    try {
      if (!microsoftChallenge.value) {
        microsoftChallenge.value = await backend.startMicrosoftLogin()
        notice.value = `请访问 ${microsoftChallenge.value.verificationUri} 并输入代码 ${microsoftChallenge.value.userCode}`
        await new Promise(resolve => setTimeout(resolve, 50))
        if (window.confirm('验证码已生成。是否自动打开 Microsoft 授权网页？')) {
          await backend.openMicrosoftLogin(microsoftChallenge.value.verificationUri)
        }
      }
      account.value = await backend.finishMicrosoftLogin()
      settings.value.accountMode = 'microsoft'
      microsoftChallenge.value = null
      await backend.saveSettings({ ...settings.value })
      notice.value = `Microsoft 正版已登录：${account.value.playerName}`
    } catch (e) { fail(e) }
    finally { loggingIn.value = false }
  }
  async function openMicrosoftPage() {
    if (!microsoftChallenge.value) return
    try { await backend.openMicrosoftLogin(microsoftChallenge.value.verificationUri) } catch (e) { fail(e) }
  }
  async function selectInstance(name: string) {
    if (name === '__empty__') {
      emptyInstanceSelected.value = true
      notice.value = '已切换到空实例：资源搜索将显示全部兼容版本'
      return
    }
    if (!settings.value.instances.some(instance => instance.name === name)) return
    emptyInstanceSelected.value = false
    settings.value.selectedInstance = name
    notice.value = `已切换到实例：${name}`
    if (!desktop) return
    try { await backend.saveSettings({ ...settings.value }) }
    catch (e) { fail(e) }
  }
  async function deleteInstance(name: string) {
    if (installing.value || resourceDownloading.value || gameRunning.value) {
      fail('下载或游戏运行时不能删除实例')
      return
    }
    error.value = ''
    notice.value = ''
    try {
      const loaded = await backend.deleteInstance(name)
      settings.value = { ...loaded, instances: loaded.instances ?? [], selectedInstance: loaded.selectedInstance ?? null }
      installed.value = settings.value.instances.map(instance => instance.name)
      account.value = await backend.loadAccount()
      emptyInstanceSelected.value = !settings.value.selectedInstance
      notice.value = `实例“${name}”及其文件已删除`
      log(notice.value)
    } catch (e) { fail(e) }
  }
  async function inspectModpack() {
    if (inspectingModpack.value) return
    inspectingModpack.value = true; error.value = ''; modpack.value = null
    try {
      const path = await backend.pickModpack()
      if (!path) return
      modpackPath.value = path
      modpack.value = await backend.inspectModpack(path)
      const baseName = (modpack.value.name || '整合包实例').replace(/[<>:"/\\|?*\u0000-\u001F]/g, ' ').replace(/[. ]+$/g, '').trim().slice(0, 36) || '整合包实例'
      let suggested = baseName
      let suffix = 1
      while (settings.value.instances.some(instance => instance.name.toLocaleLowerCase() === suggested.toLocaleLowerCase())) suggested = `${baseName.slice(0, 32)} (${++suffix})`
      modpackInstanceName.value = suggested
      const base = settings.value.gameDir.replace(/[\\/]+$/, '')
      modpackInstancePath.value = base ? `${base}\\${suggested}` : ''
      log(`已识别 ${modpack.value.format} 整合包：${modpack.value.name || path}`)
    } catch (e) { fail(e) }
    finally { inspectingModpack.value = false }
  }
  async function browseModpackInstancePath() {
    try {
      const path = await backend.pickDirectory(modpackInstancePath.value || settings.value.gameDir)
      if (path) modpackInstancePath.value = path
    } catch (e) { fail(e) }
  }
  async function importModpackAsInstance() {
    if (!modpack.value || !modpackPath.value || installing.value || resourceDownloading.value) return
    const name = modpackInstanceName.value.trim()
    const path = modpackInstancePath.value.trim()
    let unlistenGame: (() => void) | undefined
    let unlistenPack: (() => void) | undefined
    try {
      const version = modpack.value.minecraftVersion
      if (!version) throw new Error('整合包未声明 Minecraft 版本，无法自动创建实例')
      if (!name || name.length > 40) throw new Error('实例名称须为 1–40 个字符')
      if (settings.value.instances.some(instance => instance.name.toLocaleLowerCase() === name.toLocaleLowerCase())) throw new Error(`实例名称不能重复：${name}`)
      if (!/^(?:[A-Za-z]:[\\/]|\\\\)/.test(path)) throw new Error('请选择绝对实例目录')
      const normalized = path.replaceAll('/', '\\').replace(/[\\]+$/, '').toLocaleLowerCase()
      if (settings.value.instances.some(instance => {
        const existing = instance.path.replaceAll('/', '\\').replace(/[\\]+$/, '').toLocaleLowerCase()
        return existing === normalized || existing.startsWith(`${normalized}\\`) || normalized.startsWith(`${existing}\\`)
      })) throw new Error('每个实例必须使用互不嵌套的独立文件夹')
      startDownloadTask(name, `正在准备 Minecraft ${version}…`)
      unlistenGame = await backend.onProgress(value => updateDownloadTask(
        value.total ? Math.round(value.completed / value.total * 100) : 0,
        value.message,
        value.total ? `${value.completed} / ${value.total} 个游戏文件` : '',
      ))
      unlistenPack = await backend.onModpackProgress(value => updateDownloadTask(
        value.total ? Math.round(value.completed / value.total * 100) : 0,
        value.message,
        value.total ? `${value.completed} / ${value.total} 个应用步骤` : '',
      ))
      installing.value = true
      await backend.install(version, path)
      const created: typeof settings.value.instances[number] = { name, version, path }
      const loaderSpec = modpack.value.loaders.find(value => /^(fabric-loader|fabric|forge|neoforge)\s+/i.test(value))
      if (loaderSpec) {
        const separator = loaderSpec.indexOf(' ')
        const rawLoader = loaderSpec.slice(0, separator).toLowerCase()
        const loader = rawLoader === 'fabric-loader' ? 'fabric' : rawLoader
        const loaderVersion = loaderSpec.slice(separator + 1).trim()
        if (loader === 'fabric' || loader === 'forge' || loader === 'neoforge') {
          updateDownloadTask(0, `正在安装 ${loader} ${loaderVersion}…`)
          const installedLoader = await backend.installLoader(path, version, loader, loaderVersion, settings.value.javaPath)
          created.loader = `${installedLoader.loader} ${installedLoader.loaderVersion}`
          created.launchVersion = installedLoader.launchVersion
        }
      }
      settings.value.instances.push(created)
      settings.value.selectedInstance = name
      settings.value.selectedVersion = version
      emptyInstanceSelected.value = false
      await backend.saveSettings({ ...settings.value })
      const result = await backend.applyModpack(modpackPath.value, path, version)
      finishDownloadTask('整合包实例安装完成', `应用 ${result.appliedFiles + result.downloadedFiles} 个文件`)
      notice.value = `${name} 已安装并应用整合包`
    } catch (e) {
      fail(e)
      failDownloadTask(e)
    } finally {
      unlistenGame?.()
      unlistenPack?.()
      installing.value = false
    }
  }
  async function prepareLoader(loader: 'fabric' | 'forge' | 'neoforge') {
    const instance = selectedInstance.value
    if (!instance || loadingLoaderVersions.value) return
    if (instance.loader) { fail(`当前实例已经安装 ${instance.loader}，不能再添加加载器`); return }
    loaderKind.value = loader; loaderVersions.value = []; selectedLoaderVersion.value = ''; loadingLoaderVersions.value = true; error.value = ''
    try {
      loaderVersions.value = await backend.loaderVersions(instance.version, loader)
      selectedLoaderVersion.value = loaderVersions.value[0] ?? ''
    } catch (e) { fail(e) }
    finally { loadingLoaderVersions.value = false }
  }
  async function installLoader() {
    const instance = selectedInstance.value
    const loader = loaderKind.value
    if (!instance || !loader || !selectedLoaderVersion.value || installingLoader.value) return
    installingLoader.value = true; error.value = ''; notice.value = ''
    try {
      log(`正在为 ${instance.name} 安装 ${loader}`)
      const result = await backend.installLoader(instance.path, instance.version, loader, selectedLoaderVersion.value, settings.value.javaPath)
      instance.loader = `${result.loader} ${result.loaderVersion}`
      instance.launchVersion = result.launchVersion
      await backend.saveSettings({ ...settings.value })
      notice.value = `${instance.name} 已安装 ${instance.loader}`; log(notice.value)
    } catch (e) { fail(e) }
    finally { installingLoader.value = false }
  }
  async function applyModpack() {
    const instance = selectedInstance.value
    if (!instance || !modpack.value || !modpackPath.value || applyingModpack.value) return
    applyingModpack.value = true; error.value = ''; notice.value = ''
    let unlisten: (() => void) | undefined
    try {
      startDownloadTask(`${modpack.value.name || '外部整合包'} → ${instance.name}`, '正在准备应用整合包…')
      unlisten = await backend.onModpackProgress(value => updateDownloadTask(
        value.total ? Math.round(value.completed / value.total * 100) : 0,
        value.message,
        value.total ? `${value.completed} / ${value.total} 个应用步骤` : '',
      ))
      const result = await backend.applyModpack(modpackPath.value, instance.path, instance.version)
      notice.value = `整合包已应用：复制 ${result.appliedFiles} 个文件，下载 ${result.downloadedFiles} 个文件`
      if (result.skippedFiles) notice.value += `，跳过 ${result.skippedFiles} 个需平台授权的文件`
      for (const warning of result.warnings) log(warning, 'error')
      log(notice.value)
      finishDownloadTask('整合包应用完成', notice.value)
    } catch (e) { fail(e); failDownloadTask(e) }
    finally { unlisten?.(); applyingModpack.value = false }
  }
  async function refresh() {
    loading.value = true; error.value = ''
    try {
      versions.value = await backend.versions()
      if (!versions.value.some(v => v.id === settings.value.selectedVersion)) settings.value.selectedVersion = versions.value.find(v => v.type === 'release')?.id ?? null
      log(`已获取 ${versions.value.length} 个官方版本`)
    } catch (e) { fail(e) } finally { loading.value = false }
  }
  async function initialize() {
    if (initialized.value) return
    initialized.value = true
    if (!desktop) { log('浏览器界面预览。版本下载、Java 检测和设置保存需要桌面版。'); return }
    try {
      const loaded = await backend.loadSettings()
      settings.value = { ...loaded, instances: loaded.instances ?? [], selectedInstance: loaded.selectedInstance ?? null }
      emptyInstanceSelected.value = !settings.value.selectedInstance
      installed.value = settings.value.instances.map(instance => instance.name)
    } catch (e) { fail(e); return }
    await loadSkin()
    await refresh()
  }
  async function save() {
    if (saving.value) return
    saving.value = true; error.value = ''; notice.value = ''
    try {
      await backend.saveSettings({ ...settings.value })
      installed.value = settings.value.instances.map(instance => instance.name)
      notice.value = '设置已保存'; log('设置已保存')
    } catch (e) { fail(e) } finally { saving.value = false }
  }
  async function checkJava() {
    checkingJava.value = true; error.value = ''; java.value = null
    try { java.value = await backend.detectJava(settings.value.javaPath); log(`Java ${java.value.major} · ${java.value.is64Bit ? '64 位' : '未识别为 64 位'}`) }
    catch (e) { fail(e) } finally { checkingJava.value = false }
  }
  async function installJava() {
    const instance = selectedInstance.value
    if (!instance || installingJava.value) return
    installingJava.value = true; error.value = ''; notice.value = ''; javaProgress.value = null
    let unlisten: (() => void) | undefined
    try {
      unlisten = await backend.onJavaProgress(progress => { javaProgress.value = progress })
      log(`正在为实例 ${instance.name} 自动配置 Java`)
      java.value = await backend.installJava(instance.path, instance.version)
      settings.value.javaPath = java.value.path
      await backend.saveSettings({ ...settings.value })
      notice.value = `Java ${java.value.major} 已安装并启用`
      log(notice.value)
    } catch (e) { fail(e) }
    finally { unlisten?.(); installingJava.value = false }
  }
  async function install() {
    if (!settings.value.selectedVersion || installing.value) return
    const name = instanceName.value.trim()
    const path = instancePath.value.trim()
    if (!name || name.length > 40) { fail('实例名称须为 1–40 个字符'); return }
    if (settings.value.instances.some(instance => instance.name.toLocaleLowerCase() === name.toLocaleLowerCase())) { fail(`实例名称不能重复：${name}`); return }
    const normalizedPath = path.replaceAll('/', '\\').replace(/[\\]+$/, '').toLocaleLowerCase()
    if (settings.value.instances.some(instance => {
      const existing = instance.path.replaceAll('/', '\\').replace(/[\\]+$/, '').toLocaleLowerCase()
      return existing === normalizedPath || existing.startsWith(`${normalizedPath}\\`) || normalizedPath.startsWith(`${existing}\\`)
    })) { fail('每个实例必须使用互不嵌套的独立文件夹'); return }
    if (!/^(?:[A-Za-z]:[\\/]|\\\\)/.test(path)) { fail('请输入绝对实例目录，例如 D:\\Minecraft\\我的世界'); return }
    installing.value = true; error.value = ''; notice.value = ''; downloadTask.value = null; page.value = 'downloads'
    const version = settings.value.selectedVersion
    progress.value = { version, completed: 0, total: 0, message: '读取版本信息…' }
    let unlisten: (() => void) | undefined
    try {
      unlisten = await backend.onProgress(p => { progress.value = p })
      await backend.saveSettings({ ...settings.value })
      log(`开始下载 / 校验 ${version}`)
      await backend.install(version, path)
      settings.value.instances.push({ name, version, path })
      settings.value.selectedInstance = name
      installed.value = settings.value.instances.map(instance => instance.name)
      await backend.saveSettings({ ...settings.value })
      notice.value = `${name}（${version}）已安装`; log(notice.value)
    } catch (e) {
      fail(e)
      if (progress.value) progress.value.message = error.value
    } finally { unlisten?.(); installing.value = false }
  }
  async function cancel() { try { await backend.cancel(); log('已请求取消，等待当前任务停止') } catch (e) { fail(e) } }
  async function launch(diagnostic = false) {
    const instance = selectedInstance.value
    if (!instance || gameRunning.value) return
    const version = instance.version
    gameRunning.value = true; error.value = ''; notice.value = ''; page.value = 'downloads'
    let unlisten: (() => void) | undefined
    try {
      unlisten = await backend.onGameEvent((event: GameEvent) => {
        log(event.message, event.kind === 'stderr' || (event.kind === 'exited' && event.exitCode !== 0) ? 'error' : 'info')
        if (event.kind === 'started') notice.value = `${version} 已启动`
      })
      await backend.saveSettings({ ...settings.value })
      log(diagnostic ? `正在以 JFR 诊断模式启动 Minecraft ${version}` : `正在以 ${settings.value.offlineName} 离线启动 Minecraft ${version}`)
      await backend.launch(version, instance.launchVersion || version, instance.path, { ...settings.value }, diagnostic)
    } catch (e) { fail(e) }
    finally { unlisten?.(); gameRunning.value = false }
  }
  return { desktop, page, settings, versions, installed, instanceName, instancePath, selectedInstance, emptyInstanceSelected, instanceSelection, modpack, modpackPath, modpackInstanceName, modpackInstancePath, inspectingModpack, scanningInstances, installingLoader, loadingLoaderVersions, loaderKind, loaderVersions, selectedLoaderVersion, applyingModpack, java, skin, account, accountUsername, accountPassword, loggingIn, microsoftChallenge, logs, loading, saving, checkingJava, installingJava, javaProgress, installing, resourceDownloading, gameRunning, error, notice, search, versionChannel, progress, downloadTask, filteredVersions, selected, percent, startDownloadTask, updateDownloadTask, finishDownloadTask, failDownloadTask, chooseVersion, browseInstancePath, browseDefaultPath, scanInstanceFolder, browseJava, loadSkin, importSkin, removeSkin, loginLittleSkin, loginMicrosoft, openMicrosoftPage, logoutAccount, browseModpackInstancePath, selectInstance, deleteInstance, inspectModpack, importModpackAsInstance, prepareLoader, installLoader, applyModpack, initialize, refresh, save, checkJava, installJava, install, cancel, launch }
})
