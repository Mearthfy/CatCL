import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import type { GameVersion, Settings, JavaInfo, InstallProgress, WorldLibrary, GameEvent, JavaProgress, ModpackInfo, LoaderInstallResult, ModpackApplyResult, ResourceProject, ResourceInstallResult, ResourceVersionInfo, ResourceProgress, ModpackProgress, SkinInfo, AccountInfo, MicrosoftChallenge, MinecraftLanStatus, P2pNetworkSnapshot, NatReport } from '../types'

export const desktop = isTauri()
function native<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktop) return Promise.reject(new Error('请通过 npm run desktop 启动桌面版以使用此功能。'))
  return invoke<T>(command, args)
}
export const backend = {
  pickDirectory: async (defaultPath?: string) => {
    if (!desktop) throw new Error('请通过桌面版打开文件夹选择器。')
    const selected = await open({ directory: true, multiple: false, defaultPath })
    return typeof selected === 'string' ? selected : null
  },
  pickJava: async (defaultPath?: string) => {
    if (!desktop) throw new Error('请通过桌面版打开文件选择器。')
    const selected = await open({ directory: false, multiple: false, defaultPath, filters: [{ name: 'Java', extensions: ['exe'] }] })
    return typeof selected === 'string' ? selected : null
  },
  pickSkin: async () => {
    if (!desktop) throw new Error('请通过桌面版打开皮肤文件选择器。')
    const selected = await open({ directory: false, multiple: false, filters: [{ name: 'Minecraft 皮肤', extensions: ['png'] }] })
    return typeof selected === 'string' ? selected : null
  },
  importSkin: (player: string, path: string) => native<SkinInfo>('import_skin', { player, path }),
  loadSkin: (player: string) => native<SkinInfo | null>('load_skin', { player }),
  removeSkin: (player: string) => native<void>('remove_skin', { player }),
  loadAccount: () => native<AccountInfo | null>('load_account'),
  loginLittleSkin: (username: string, password: string) => native<AccountInfo>('login_littleskin', { username, password }),
  startMicrosoftLogin: () => native<MicrosoftChallenge>('start_microsoft_login'),
  finishMicrosoftLogin: () => native<AccountInfo>('finish_microsoft_login'),
  openMicrosoftLogin: (url: string) => native<void>('open_microsoft_login', { url }),
  logoutAccount: () => native<void>('logout_account'),
  pickModpack: async () => {
    if (!desktop) throw new Error('请通过桌面版选择整合包。')
    const selected = await open({ directory: false, multiple: false, filters: [{ name: 'Minecraft 整合包', extensions: ['zip', 'mrpack'] }] })
    return typeof selected === 'string' ? selected : null
  },
  inspectModpack: (path: string) => native<ModpackInfo>('inspect_modpack', { path }),
  applyModpack: (path: string, instancePath: string, gameVersion: string) => native<ModpackApplyResult>('apply_modpack', { path, instancePath, gameVersion }),
  searchResources: (query: string, projectType: string, gameVersion: string, loader: string) => native<ResourceProject[]>('search_resources', { query, projectType, gameVersion, loader }),
  openResourceSite: (source: string, query: string) => native<void>('open_resource_site', { source, query }),
  resourceVersions: (projectId: string, gameVersion: string, loader: string) => native<ResourceVersionInfo[]>('resource_versions', { projectId, gameVersion, loader }),
  downloadResource: (projectId: string, versionId: string, projectType: string, destination: string) => native<ResourceInstallResult>('download_resource', { projectId, versionId, projectType, destination }),
  installResource: (projectId: string, projectType: string, instancePath: string, gameVersion: string, loader: string) => native<ResourceInstallResult>('install_resource', { projectId, projectType, instancePath, gameVersion, loader }),
  loaderVersions: (gameVersion: string, loader: string) => native<string[]>('list_loader_versions', { gameVersion, loader }),
  installLoader: (instancePath: string, gameVersion: string, loader: string, loaderVersion: string, javaPath: string) => native<LoaderInstallResult>('install_loader', { instancePath, gameVersion, loader, loaderVersion, javaPath }),
  worlds: (gameDir: string) => native<WorldLibrary>('list_worlds', { gameDir }),
  renameWorld: (gameDir: string, id: string, name: string) => native<void>('rename_world', { gameDir, id, name }),
  importWorld: (gameDir: string, source: string, name: string) => native<void>('import_world', { gameDir, source, name }),
  openWorld: (gameDir: string, id: string) => native<void>('open_world_folder', { gameDir, id }),
  loadSettings: () => native<Settings>('load_settings'),
  saveSettings: (settings: Settings) => native<void>('save_settings', { settings }),
  scanInstanceFolder: (path: string) => native<Settings['instances']>('scan_instance_folder', { path }),
  deleteInstance: (name: string) => native<Settings>('delete_instance', { name }),
  versions: () => native<GameVersion[]>('list_versions'),
  detectJava: (path: string) => native<JavaInfo>('detect_java', { path }),
  installJava: (gameDir: string, version: string) => native<JavaInfo>('install_java', { gameDir, version }),
  install: (version: string, gameDir: string) => native<void>('install_version', { version, gameDir }),
  cancel: () => native<void>('cancel_install'),
  installed: (gameDir: string) => native<string[]>('installed_versions', { gameDir }),
  launch: (baseVersion: string, launchVersion: string, instancePath: string, settings: Settings, diagnostic = false) => native<void>('launch_game', { request: { baseVersion, launchVersion, instancePath, javaPath: settings.javaPath, memoryMb: settings.memoryMb, offlineName: settings.offlineName, accountMode: settings.accountMode, diagnostic, experimentalHotset: settings.experimentalHotset } }),
  detectMinecraftLan: (instancePath: string, timeoutSecs = 120) => native<MinecraftLanStatus>('detect_minecraft_lan', { instancePath, timeoutSecs }),
  inspectP2pNetwork: (port = 0) => native<P2pNetworkSnapshot>('inspect_p2p_network', { port }),
  detectP2pNat: (stunServers: string[]) => native<NatReport>('detect_p2p_nat', { stunServers }),
  onProgress: (callback: (progress: InstallProgress) => void) => listen<InstallProgress>('install-progress', event => callback(event.payload)),
  onResourceProgress: (callback: (progress: ResourceProgress) => void) => listen<ResourceProgress>('resource-progress', event => callback(event.payload)),
  onModpackProgress: (callback: (progress: ModpackProgress) => void) => listen<ModpackProgress>('modpack-progress', event => callback(event.payload)),
  onGameEvent: (callback: (event: GameEvent) => void) => listen<GameEvent>('game-event', event => callback(event.payload)),
  onJavaProgress: (callback: (progress: JavaProgress) => void) => listen<JavaProgress>('java-progress', event => callback(event.payload)),
  onMinecraftLan: (callback: (status: MinecraftLanStatus) => void) => listen<MinecraftLanStatus>('p2p-minecraft-lan', event => callback(event.payload)),
}
