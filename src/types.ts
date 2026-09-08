export type Page = 'home' | 'versions' | 'resources' | 'worlds' | 'multiplayer' | 'downloads' | 'settings'
export interface WorldInfo { id: string; name: string; folder: string; location: string; modifiedAt: number }
export interface WorldLibrary { worlds: WorldInfo[]; warnings: string[] }
export interface GameVersion { id: string; type: string; releaseTime: string }
export interface GameInstance { name: string; version: string; path: string; loader?: string | null; launchVersion?: string | null }
export interface LoaderInstallResult { loader: string; loaderVersion: string; launchVersion: string }
export interface ModpackInfo { format: string; name: string; version: string; minecraftVersion: string; loaders: string[]; fileCount: number; warnings: string[] }
export interface ModpackApplyResult { appliedFiles: number; downloadedFiles: number; skippedFiles: number; warnings: string[] }
export interface ModpackProgress { completed: number; total: number; message: string }
export interface ResourceProject { id: string; title: string; description: string; projectType: string; iconUrl: string | null; downloads: number; author: string }
export interface ResourceInstallResult { path: string; projectType: string }
export interface ResourceProgress { id: string; downloaded: number; total: number; message: string }
export interface DownloadTask { title: string; percent: number; message: string; detail: string; status: 'active' | 'complete' | 'error' }
export interface ResourceDependency { projectId: string | null; versionId: string | null; dependencyType: string; title: string }
export interface ResourceVersionInfo { id: string; name: string; versionNumber: string; gameVersions: string[]; loaders: string[]; datePublished: string; dependencies: ResourceDependency[] }
export type AccountMode = 'offline' | 'littleskin' | 'microsoft'
export interface AccountInfo { mode: AccountMode; playerName: string; uuid: string }
export interface MicrosoftChallenge { userCode: string; verificationUri: string; expiresIn: number }
export interface Settings { javaPath: string; offlineName: string; accountMode: AccountMode; microsoftClientId: string; memoryMb: number; gameDir: string; selectedVersion: string | null; showSnapshots: boolean; instances: GameInstance[]; selectedInstance: string | null; experimentalHotset?: boolean }
export interface JavaInfo { path: string; version: string; major: number; is64Bit: boolean }
export interface JavaProgress { major: number; downloaded: number; total: number; message: string }
export interface InstallProgress { version: string; completed: number; total: number; message: string }
export interface GameEvent { version: string; kind: 'started' | 'stdout' | 'stderr' | 'exited'; message: string; exitCode: number | null }
export interface LogEntry { time: string; message: string; level: 'info' | 'error' }
export interface SkinInfo { player: string; width: number; height: number; dataUrl: string }
export interface MinecraftLanStatus { port: number; address: string; ready: boolean }
export type P2pCandidateKind = 'Loopback' | 'Local' | 'Ipv6Direct' | 'Ipv4Direct' | 'Mapped'
export interface P2pCandidate { address: string; kind: P2pCandidateKind }
export interface P2pNetworkSnapshot { ipv6Direct: boolean; udpAvailable: boolean; candidates: P2pCandidate[] }
export type NatType = 'OpenInternet' | 'ConeOrRestricted' | 'Symmetric' | 'Unknown'
export interface NatReport { natType: NatType; mappedAddresses: string[]; successfulServers: number }
export type MappingMethod = 'Upnp' | 'NatPmp' | 'Pcp'
export interface PortMapping { method: MappingMethod; internalPort: number; externalPort: number; externalAddress: string | null; lifetimeSecs: number }
export type P2pSessionState = 'Idle' | 'CreatingSession' | 'DetectingNetwork' | 'WaitingForAnswer' | 'Connecting' | 'HolePunching' | 'Handshaking' | 'Connected' | 'MinecraftReady' | 'Closing' | 'Closed' | 'Failed'
export interface P2pRoomSnapshot { role: 'none' | 'host' | 'client'; state: P2pSessionState; inviteCode: string | null; answerCode: string | null; minecraftLanPort: number | null; localAddress: string | null; playerCount: number; transport: string | null; message: string }
