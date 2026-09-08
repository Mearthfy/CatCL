import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { backend } from '../services/backend'
import { useLauncherStore } from './launcher'

vi.mock('../services/backend', () => ({ desktop: true, backend: {
  loadSettings: vi.fn(), saveSettings: vi.fn(), deleteInstance: vi.fn(), versions: vi.fn(), installed: vi.fn(),
  detectJava: vi.fn(), install: vi.fn(), cancel: vi.fn(), onProgress: vi.fn(),
  launch: vi.fn(), onGameEvent: vi.fn(), installJava: vi.fn(), onJavaProgress: vi.fn(),
  pickDirectory: vi.fn(), pickJava: vi.fn(), pickSkin: vi.fn(), importSkin: vi.fn(), loadSkin: vi.fn(), removeSkin: vi.fn(), pickModpack: vi.fn(), inspectModpack: vi.fn(), loaderVersions: vi.fn(), installLoader: vi.fn(), applyModpack: vi.fn(), onModpackProgress: vi.fn(),
  loadAccount: vi.fn(), loginLittleSkin: vi.fn(), logoutAccount: vi.fn(), scanInstanceFolder: vi.fn(),
  startMicrosoftLogin: vi.fn(), finishMicrosoftLogin: vi.fn(),
  openMicrosoftLogin: vi.fn(),
} }))

const versions = [
  { id: '1.21.1', type: 'release', releaseTime: '2024-08-08T00:00:00Z' },
  { id: '24w33a', type: 'snapshot', releaseTime: '2024-08-15T00:00:00Z' },
]

describe('launcher state', () => {
  beforeEach(() => { setActivePinia(createPinia()); vi.resetAllMocks() })

  it('selects a real release and filters snapshots and search terms', async () => {
    vi.mocked(backend.versions).mockResolvedValue(versions)
    const store = useLauncherStore()
    await store.refresh()
    expect(store.settings.selectedVersion).toBe('1.21.1')
    expect(store.filteredVersions).toHaveLength(1)
    store.versionChannel = 'snapshot'
    store.search = '24W'
    expect(store.filteredVersions.map(v => v.id)).toEqual(['24w33a'])
  })

  it('releases the busy state after network failure without inventing versions', async () => {
    vi.mocked(backend.versions).mockRejectedValue(new Error('network unavailable'))
    const store = useLauncherStore()
    await store.refresh()
    expect(store.loading).toBe(false)
    expect(store.versions).toEqual([])
    expect(store.error).toBe('network unavailable')
  })

  it('cleans the event listener and does not mark a failed installation complete', async () => {
    const unlisten = vi.fn()
    vi.mocked(backend.onProgress).mockResolvedValue(unlisten)
    vi.mocked(backend.install).mockRejectedValue('download cancelled')
    const store = useLauncherStore()
    store.settings.selectedVersion = '1.21.1'
    store.instanceName = '测试实例'
    store.instancePath = 'D:\\Minecraft\\test-instance'
    await store.install()
    expect(store.installing).toBe(false)
    expect(store.error).toBe('download cancelled')
    expect(store.installed).toEqual([])
    expect(unlisten).toHaveBeenCalledOnce()
    expect(backend.installed).not.toHaveBeenCalled()
  })

  it('subscribes before installation and refreshes installed versions after success', async () => {
    const unlisten = vi.fn()
    vi.mocked(backend.onProgress).mockResolvedValue(unlisten)
    vi.mocked(backend.installed).mockResolvedValue(['1.21.1'])
    const store = useLauncherStore()
    store.settings.selectedVersion = '1.21.1'
    store.instanceName = '生存世界'
    store.instancePath = 'D:\\Minecraft\\survival'
    await store.install()
    expect(vi.mocked(backend.onProgress).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(backend.install).mock.invocationCallOrder[0]!)
    expect(store.installed).toEqual(['生存世界'])
    expect(store.settings.instances).toEqual([{ name: '生存世界', version: '1.21.1', path: 'D:\\Minecraft\\survival' }])
    expect(store.notice).toContain('已安装')
    expect(unlisten).toHaveBeenCalledOnce()
  })

  it('allows the same Minecraft version twice but rejects duplicate instance names', async () => {
    const store = useLauncherStore()
    store.settings.selectedVersion = '1.21.1'
    store.settings.instances = [{ name: '生存世界', version: '1.21.1', path: 'D:\\Minecraft\\one' }]
    store.instanceName = '生存世界'
    store.instancePath = 'D:\\Minecraft\\two'
    await store.install()
    expect(store.error).toContain('名称不能重复')
    expect(backend.install).not.toHaveBeenCalled()

    store.instanceName = '创造世界'
    await store.install()
    expect(backend.install).toHaveBeenCalledWith('1.21.1', 'D:\\Minecraft\\two')
  })

  it('switches the active instance and persists the selection', async () => {
    const store = useLauncherStore()
    store.settings.instances = [
      { name: '生存', version: '1.21.1', path: 'D:\\Minecraft\\survival' },
      { name: '创造', version: '1.21.1', path: 'D:\\Minecraft\\creative' },
    ]
    await store.selectInstance('创造')
    expect(store.selectedInstance?.path).toBe('D:\\Minecraft\\creative')
    expect(backend.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ selectedInstance: '创造' }))
  })

  it('removes a deleted instance from local state', async () => {
    vi.mocked(backend.deleteInstance).mockResolvedValue({
      javaPath: 'java', offlineName: 'Player', accountMode: 'offline', microsoftClientId: '', memoryMb: 4096, gameDir: 'D:\\Minecraft',
      selectedVersion: '1.21.1', showSnapshots: false, selectedInstance: null, instances: [],
    })
    const store = useLauncherStore()
    store.settings.instances = [{ name: '待删除', version: '1.21.1', path: 'D:\\Minecraft\\old' }]
    store.settings.selectedInstance = '待删除'
    await store.deleteInstance('待删除')
    expect(backend.deleteInstance).toHaveBeenCalledWith('待删除')
    expect(store.settings.instances).toEqual([])
    expect(store.emptyInstanceSelected).toBe(true)
  })

  it('selects and recognizes a modpack archive', async () => {
    vi.mocked(backend.pickModpack).mockResolvedValue('D:\\Packs\\example.mrpack')
    vi.mocked(backend.inspectModpack).mockResolvedValue({ format: 'Modrinth', name: 'Example', version: '1.0', minecraftVersion: '1.21.1', loaders: ['fabric-loader 0.16.0'], fileCount: 42, warnings: [] })
    const store = useLauncherStore()
    await store.inspectModpack()
    expect(backend.inspectModpack).toHaveBeenCalledWith('D:\\Packs\\example.mrpack')
    expect(store.modpack?.minecraftVersion).toBe('1.21.1')
    expect(store.inspectingModpack).toBe(false)
  })

  it('installs a loader into the selected instance and records its launch profile', async () => {
    vi.mocked(backend.loaderVersions).mockResolvedValue(['0.16.0', '0.15.0'])
    vi.mocked(backend.installLoader).mockResolvedValue({ loader: 'Fabric', loaderVersion: '0.16.0', launchVersion: 'fabric-loader-0.16.0-1.21.1' })
    const store = useLauncherStore()
    store.settings.instances = [{ name: '模组实例', version: '1.21.1', path: 'D:\\Minecraft\\modded' }]
    store.settings.selectedInstance = '模组实例'
    await store.prepareLoader('fabric')
    store.selectedLoaderVersion = '0.16.0'
    await store.installLoader()
    expect(backend.loaderVersions).toHaveBeenCalledWith('1.21.1', 'fabric')
    expect(backend.installLoader).toHaveBeenCalledWith('D:\\Minecraft\\modded', '1.21.1', 'fabric', '0.16.0', 'java')
    expect(store.selectedInstance?.loader).toBe('Fabric 0.16.0')
    expect(store.selectedInstance?.launchVersion).toBe('fabric-loader-0.16.0-1.21.1')
    expect(backend.saveSettings).toHaveBeenCalled()
  })

  it('applies a recognized pack to the selected instance', async () => {
    vi.mocked(backend.onModpackProgress).mockResolvedValue(vi.fn())
    vi.mocked(backend.applyModpack).mockResolvedValue({ appliedFiles: 5, downloadedFiles: 12, skippedFiles: 0, warnings: [] })
    const store = useLauncherStore()
    store.settings.instances = [{ name: '模组实例', version: '1.21.1', path: 'D:\\Minecraft\\modded' }]
    store.settings.selectedInstance = '模组实例'
    store.modpackPath = 'D:\\Packs\\pack.mrpack'
    store.modpack = { format: 'Modrinth', name: 'Pack', version: '1', minecraftVersion: '1.21.1', loaders: ['fabric-loader'], fileCount: 17, warnings: [] }
    await store.applyModpack()
    expect(backend.applyModpack).toHaveBeenCalledWith('D:\\Packs\\pack.mrpack', 'D:\\Minecraft\\modded', '1.21.1')
    expect(store.notice).toContain('复制 5 个文件，下载 12 个文件')
  })

  it('imports an external modpack with its game version and loader', async () => {
    vi.mocked(backend.onProgress).mockResolvedValue(vi.fn())
    vi.mocked(backend.onModpackProgress).mockResolvedValue(vi.fn())
    vi.mocked(backend.installLoader).mockResolvedValue({ loader: 'Fabric', loaderVersion: '0.16.0', launchVersion: 'fabric-loader-0.16.0-1.20.1' })
    vi.mocked(backend.applyModpack).mockResolvedValue({ appliedFiles: 3, downloadedFiles: 7, skippedFiles: 0, warnings: [] })
    const store = useLauncherStore()
    store.modpack = { format: 'Modrinth', name: 'Adventure', version: '1', minecraftVersion: '1.20.1', loaders: ['fabric-loader 0.16.0'], fileCount: 10, warnings: [] }
    store.modpackPath = 'D:\\Packs\\adventure.mrpack'
    store.modpackInstanceName = 'Adventure'
    store.modpackInstancePath = 'D:\\Minecraft\\Adventure'
    await store.importModpackAsInstance()
    expect(backend.install).toHaveBeenCalledWith('1.20.1', 'D:\\Minecraft\\Adventure')
    expect(backend.installLoader).toHaveBeenCalledWith('D:\\Minecraft\\Adventure', '1.20.1', 'fabric', '0.16.0', 'java')
    expect(store.settings.instances[0]?.launchVersion).toBe('fabric-loader-0.16.0-1.20.1')
    expect(store.downloadTask?.status).toBe('complete')
  })

  it('does not retain a stale Java detection after a failed check', async () => {
    const store = useLauncherStore()
    store.java = { path: 'java', version: '21', major: 21, is64Bit: true }
    vi.mocked(backend.detectJava).mockRejectedValue('not found')
    await store.checkJava()
    expect(store.java).toBeNull()
    expect(store.checkingJava).toBe(false)
  })

  it('streams game output and releases running state after exit', async () => {
    const unlisten = vi.fn()
    vi.mocked(backend.onGameEvent).mockImplementation(async callback => {
      callback({ version: '1.21.1', kind: 'started', message: 'game started', exitCode: null })
      callback({ version: '1.21.1', kind: 'stdout', message: 'Setting user: Player', exitCode: null })
      callback({ version: '1.21.1', kind: 'exited', message: 'game exited', exitCode: 0 })
      return unlisten
    })
    const store = useLauncherStore()
    store.settings.instances = [{ name: '生存世界', version: '1.21.1', path: 'D:\\Minecraft\\survival' }]
    store.settings.selectedInstance = '生存世界'
    await store.launch()
    expect(backend.launch).toHaveBeenCalledWith('1.21.1', '1.21.1', 'D:\\Minecraft\\survival', expect.objectContaining({ memoryMb: 4096 }), false)
    expect(store.logs.map(entry => entry.message)).toEqual(expect.arrayContaining(['game started', 'Setting user: Player', 'game exited']))
    expect(store.gameRunning).toBe(false)
    expect(unlisten).toHaveBeenCalledOnce()
  })

  it('installs matching Java, selects it and persists the path', async () => {
    const unlisten = vi.fn()
    vi.mocked(backend.onJavaProgress).mockImplementation(async callback => {
      callback({ major: 21, downloaded: 50, total: 100, message: 'downloading' })
      return unlisten
    })
    vi.mocked(backend.installJava).mockResolvedValue({ path: 'D:\\CatCL\\java.exe', version: 'Temurin 21', major: 21, is64Bit: true })
    const store = useLauncherStore()
    store.settings.instances = [{ name: '生存世界', version: '1.21.1', path: 'D:\\Minecraft\\survival' }]
    store.settings.selectedInstance = '生存世界'
    await store.installJava()
    expect(store.settings.javaPath).toBe('D:\\CatCL\\java.exe')
    expect(store.javaProgress?.downloaded).toBe(50)
    expect(backend.saveSettings).toHaveBeenCalled()
    expect(store.installingJava).toBe(false)
    expect(unlisten).toHaveBeenCalledOnce()
  })
})
