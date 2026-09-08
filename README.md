# CatCL (CCL)

面向 Windows x64 的 Minecraft Java 版启动器，当前为开发预览。

## 技术栈

- Rust：网络请求、并发下载、SHA-1 校验、Java 进程检测、设置持久化。
- Tauri 2：原生桌面窗口、命令调用和下载进度事件。
- Vue 3 + TypeScript：组合式 API、类型化前后端接口。
- Pinia：版本、设置、下载任务、日志状态。
- SCSS + Vite：界面样式和前端构建。

## 开发

需要 Node.js 22.12+（或较新 LTS）、Rust stable、Visual Studio C++ Build Tools 的桌面 C++ 工作负载、Windows SDK 和 WebView2 Runtime。

```powershell
npm install
npm run desktop
```

仅预览页面：`npm run dev`，访问 `http://127.0.0.1:1420`。浏览器中明确禁用桌面操作，不提供虚构版本或模拟下载。

```powershell
npm run build
npm test
npm run test:e2e
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml official_manifest_and_metadata_smoke -- --ignored
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run desktop:build
```

当前禁用安装包生成，`desktop:build` 生成 `src-tauri/target/release/catcl.exe`。发行安装包、签名和自动更新待后续配置。

本次已构建可直接运行的调试版：`src-tauri/target/debug/catcl.exe`。重新生成：`npm run tauri -- build --debug --no-bundle`。

浏览器交互测试使用本机 Microsoft Edge；截图保存到 `artifacts/`。官方清单冒烟测试需联网，默认 Rust 测试不会访问外网。

### 当前验证结果

- 前端生产构建通过；Pinia 5 项测试通过。
- Rust 8 项测试通过，包含真实 Mojang 清单与版本元数据校验；Clippy 严格检查通过。
- Edge 浏览器 2 项交互测试通过，覆盖页面导航、桌面功能禁用提示与最小窗口宽度布局。
- Windows 调试版构建成功。尝试桌面自动化时，当前 WebView2 环境未启用请求的 CDP 调试端口，真实窗口内 IPC 自动验证尚未完成。
- 未进行完整游戏资源下载实测；下载链路使用本地 HTTP 测试验证流式写入、校验、缓存复用和取消。

## 已实现

- 按用户 Figma 设计实现的蓝绿渐变首页、版本管理、下载任务和设置页。
- 800 × 600 无边框圆角窗口，顶部可拖动，最小化及关闭按钮已接入 Tauri；浏览器预览中禁用窗口操作。
- 从 Mojang 官方清单获取 1.13 及以上正式版、后续快照，支持搜索和筛选。
- Rust 下载游戏客户端、依赖库、资源索引、资源文件和日志配置，解压 Windows 本地 DLL。
- 8 路并发、流式文件写入、SHA-1 校验、失败最多重试 3 次、取消任务；重新下载时复用已校验文件。未实现单文件 HTTP Range 断点续传。
- Java 路径检测和版本解析，内存和游戏目录配置、本地设置保存。
- 下载事件驱动的 Pinia 状态和活动日志。
- 离线启动已完整下载的原版正式版或快照：校验玩家名，生成与 Java 版兼容的稳定离线 UUID，检查版本要求的 Java 主版本，组装官方 JVM / 游戏参数和类路径，并实时回传标准输出、错误输出与退出码。
- 独立实例：每次下载可指定唯一实例名和电脑上的绝对文件夹；同一个 Minecraft 版本可安装多份，各自保存资源、配置、模组与世界。
- 模组加载器：可为当前实例从 Fabric、Forge 或 NeoForge 官方服务选择兼容版本，运行官方安装器，并保存对应启动配置。
- 整合包应用：支持识别和应用 Modrinth、CurseForge、PCL/MCBBS、Prism/MultiMC 和通用 ZIP；Modrinth 清单文件会下载并校验 SHA-1。
- 资源广场：通过 Modrinth 官方 API，按当前实例的 Minecraft 与加载器版本搜索并安装模组、整合包、光影包、资源包和数据包。
- 根据所选且已下载的 Minecraft 元数据判断 Java 主版本，通过 Eclipse Adoptium 官方 API 自动下载 Windows x64 Temurin JRE，使用 SHA-256 校验后解压到 CatCL 应用数据目录并自动启用。不会修改系统 Java。
- 版本管理将原版正式版、原版快照分开显示；Forge、Fabric、NeoForge 使用独立入口，当前保留为后续安装器模块。
- 世界管理：扫描公共及独立实例的 `saves` 存档、搜索、打开文件夹、导入已解压地图，并为世界设置启动器显示名称。
- 世界导入采用临时目录复制后再落盘，不覆盖已有存档；拒绝路径穿越、符号链接和目录联接。自定义名称写入世界目录内的 `.verdant-world.json`，不改动 `level.dat`。

## 尚未实现

微软 OAuth / Xbox / Minecraft 正版身份验证、CurseForge 授权资源下载、ZIP 世界直接解压和自动更新仍待接入。当前支持自动安装 Java、原版离线启动、独立实例、三类模组加载器、PCL/Modrinth 等整合包，以及 Modrinth 资源广场；需要正版验证的服务器要等微软账号接入。

默认游戏数据位于 Tauri 应用数据目录下的 `minecraft`，设置位于应用配置目录的 `settings.json`，自动下载的 Java 位于应用数据目录的 `runtimes`。不会读取或改动官方启动器的 `.minecraft`，也不会修改系统 Java 或 `PATH`。

## 结构

```text
src/
  App.vue                 页面与交互
  components/WindowControls.vue  自定义窗口按钮
  components/WorldManager.vue    世界扫描、导入和命名界面
  assets/figma/           Figma 原始导出素材
  stores/launcher.ts      Pinia 状态与操作
  services/backend.ts     Tauri IPC 边界
  styles/main.scss        SCSS 主题与布局
  types.ts                前后端数据类型
src-tauri/
  src/settings.rs         设置与 Java 检测
  src/downloads.rs        清单、下载校验、解压、取消
  src/launch.rs           Java 校验、参数组装、游戏进程和日志
  src/worlds.rs           世界扫描、复制导入、命名和目录访问
  src/lib.rs              命令注册与共享状态
  tauri.conf.json         桌面配置
```

参考：[Tauri Vite 集成](https://v2.tauri.app/start/frontend/vite/)、[Tauri Rust 命令](https://v2.tauri.app/develop/calling-rust/)、[Mojang 版本清单](https://piston-meta.mojang.com/mc/game/version_manifest_v2.json)。

非 Minecraft 官方产品，与 Mojang 或 Microsoft 无隶属关系。

界面来源：[用户 Figma 画板](https://www.figma.com/design/2JGeJzMs9yfqwqIZRd7b8Z/Untitled?node-id=3-60)。设计稿中的空白区域沿用启动器现有文字和功能，未在 Figma 文件中进行修改。
