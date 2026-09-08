use crate::{
    accounts,
    downloads::{allowed, safe_join},
    settings::{parse_java_major, validate_offline_name},
    skins,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::{Emitter, State};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

#[path = "perf/benchmark.rs"]
mod benchmark;
#[path = "launch/cache.rs"]
mod cache;
#[path = "experimental/hotset.rs"]
mod hotset;
#[path = "launch/profile.rs"]
mod profile;
#[path = "launch/session.rs"]
mod session;
#[path = "perf/timeline.rs"]
mod timeline;
#[path = "perf/trace.rs"]
mod trace;
#[path = "platform/windows/usn.rs"]
mod usn;

#[derive(Default)]
pub struct GameState {
    running: AtomicBool,
}

impl GameState {
    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameEvent {
    version: String,
    kind: String,
    message: String,
    exit_code: Option<i32>,
    session_id: Option<String>,
    relative_ns: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRequest {
    base_version: String,
    launch_version: String,
    instance_path: String,
    java_path: String,
    memory_mb: u32,
    offline_name: String,
    #[serde(default = "default_account_mode")]
    account_mode: String,
    #[serde(default)]
    diagnostic: bool,
    #[serde(default)]
    experimental_hotset: bool,
}

fn default_account_mode() -> String { "offline".into() }

fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 100
        && !version.contains(['/', '\\', ':'])
        && version != "."
        && version != ".."
}

fn maven_library_path(name: &str) -> Option<String> {
    let parts: Vec<_> = name.split(':').collect();
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let classifier = parts
        .get(3)
        .map_or(String::new(), |value| format!("-{value}"));
    Some(format!(
        "{}/{}/{}/{}-{}{}.jar",
        parts[0].replace('.', "/"),
        parts[1],
        parts[2],
        parts[1],
        parts[2],
        classifier
    ))
}

fn library_key(library: &Value) -> String {
    if let Some(name) = library["name"].as_str() {
        let parts: Vec<_> = name.split(':').collect();
        if parts.len() >= 3 {
            return format!(
                "{}:{}:{}",
                parts[0],
                parts[1],
                parts.get(3).copied().unwrap_or_default()
            );
        }
        return name.to_owned();
    }
    library["downloads"]["artifact"]["path"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn merge_libraries(parent: &mut Value, child: &Value) -> Result<(), String> {
    let target = parent["libraries"]
        .as_array_mut()
        .ok_or("基础版本依赖格式错误")?;
    for library in child["libraries"].as_array().into_iter().flatten() {
        let key = library_key(library);
        if let Some(index) = target
            .iter()
            .position(|current| library_key(current) == key)
        {
            target[index] = library.clone();
        } else {
            target.push(library.clone());
        }
    }
    Ok(())
}

async fn load_metadata(root: &std::path::Path, base: &str, launch: &str) -> Result<Value, String> {
    let read = |id: &str| safe_join(root, &format!("versions/{id}/{id}.json"));
    let parent: Value = serde_json::from_slice(
        &tokio::fs::read(read(base)?)
            .await
            .map_err(|e| format!("无法读取基础版本信息：{e}"))?,
    )
    .map_err(|e| e.to_string())?;
    if base == launch {
        return Ok(parent);
    }
    let child: Value = serde_json::from_slice(
        &tokio::fs::read(read(launch)?)
            .await
            .map_err(|e| format!("无法读取模组加载器版本信息：{e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let mut merged = parent;
    for key in ["id", "mainClass", "type"] {
        if !child[key].is_null() {
            merged[key] = child[key].clone();
        }
    }
    for section in ["jvm", "game"] {
        if let Some(values) = child["arguments"][section].as_array() {
            merged["arguments"][section]
                .as_array_mut()
                .ok_or("基础版本参数格式错误")?
                .extend(values.clone());
        }
    }
    merge_libraries(&mut merged, &child)?;
    Ok(merged)
}

fn add_optimized_jvm_args(args: &mut Vec<String>, java: u32, modded: bool) {
    if !modded {
        return;
    }
    let candidates = [
        "-XX:+UseG1GC",
        "-XX:+ParallelRefProcEnabled",
        "-XX:+DisableExplicitGC",
        "-XX:MaxGCPauseMillis=150",
        "-XX:G1ReservePercent=15",
        "-XX:+UnlockExperimentalVMOptions",
        "-XX:G1NewSizePercent=15",
        "-XX:G1MaxNewSizePercent=35",
        "-XX:G1HeapRegionSize=8M",
        "-Dsun.rmi.dgc.server.gcInterval=2147483646",
        "-Dsun.rmi.dgc.client.gcInterval=2147483646",
    ];
    for candidate in candidates {
        let key = candidate
            .split('=')
            .next()
            .unwrap_or(candidate)
            .trim_start_matches("-XX:+")
            .trim_start_matches("-XX:-");
        let already_set = args.iter().any(|argument| {
            argument
                .split('=')
                .next()
                .unwrap_or(argument)
                .trim_start_matches("-XX:+")
                .trim_start_matches("-XX:-")
                == key
        });
        if !already_set {
            args.push(candidate.into());
        }
    }
    if java >= 9
        && !args
            .iter()
            .any(|argument| argument.contains("UseStringDeduplication"))
    {
        args.push("-XX:+UseStringDeduplication".into());
    }
}

fn offline_uuid(name: &str) -> String {
    let mut bytes = md5::compute(format!("OfflinePlayer:{name}")).0;
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

fn rule_matches(rule: &Value, demo: bool) -> bool {
    if let Some(os) = rule.get("os") {
        if os["name"].as_str().is_some_and(|name| name != "windows") {
            return false;
        }
        if os["arch"]
            .as_str()
            .is_some_and(|arch| arch != "x86_64" && arch != "amd64")
        {
            return false;
        }
        // Mojang uses Windows version rules for compatibility flags. The target is
        // current Windows x64, so apply those flags instead of guessing from `OS`.
    }
    if let Some(features) = rule["features"].as_object() {
        for (key, expected) in features {
            let actual = matches!(key.as_str(), "is_demo_user") && demo;
            if expected.as_bool() != Some(actual) {
                return false;
            }
        }
    }
    true
}

fn argument_allowed(argument: &Value, demo: bool) -> bool {
    let Some(rules) = argument["rules"].as_array() else {
        return true;
    };
    let mut result = false;
    for rule in rules {
        if rule_matches(rule, demo) {
            result = rule["action"] == "allow";
        }
    }
    result
}

fn expand(value: &str, variables: &HashMap<&str, String>) -> Result<String, String> {
    let mut result = value.to_owned();
    for (name, replacement) in variables {
        result = result.replace(&format!("${{{name}}}"), replacement);
    }
    if result.contains("${") {
        return Err(format!("暂不支持启动参数：{result}"));
    }
    Ok(result)
}

fn append_arguments(
    target: &mut Vec<String>,
    values: &Value,
    variables: &HashMap<&str, String>,
    demo: bool,
) -> Result<(), String> {
    for argument in values.as_array().ok_or("版本启动参数格式错误")? {
        if let Some(value) = argument.as_str() {
            target.push(expand(value, variables)?);
        } else if argument_allowed(argument, demo) {
            if let Some(value) = argument["value"].as_str() {
                target.push(expand(value, variables)?);
            } else if let Some(items) = argument["value"].as_array() {
                for item in items {
                    target.push(expand(
                        item.as_str().ok_or("启动参数必须是字符串")?,
                        variables,
                    )?);
                }
            }
        }
    }
    Ok(())
}

async fn java_major(java_path: &str) -> Result<u32, String> {
    let mut command = tokio::process::Command::new(java_path);
    command
        .arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .map_err(|_| "Java 检测超时".to_string())?
        .map_err(|e| format!("无法执行 Java：{e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    parse_java_major(&text).ok_or_else(|| "无法识别 Java 版本".into())
}

fn emit(
    app: &tauri::AppHandle,
    version: &str,
    kind: &str,
    message: impl Into<String>,
    exit_code: Option<i32>,
) {
    let _ = app.emit(
        "game-event",
        GameEvent {
            version: version.into(),
            kind: kind.into(),
            message: message.into(),
            exit_code,
            session_id: None,
            relative_ns: None,
        },
    );
}

fn emit_timed(
    app: &tauri::AppHandle,
    version: &str,
    kind: &str,
    message: impl Into<String>,
    exit_code: Option<i32>,
    recorder: &session::StartupSession,
) {
    let _ = app.emit(
        "game-event",
        GameEvent {
            version: version.into(),
            kind: kind.into(),
            message: message.into(),
            exit_code,
            session_id: Some(recorder.id().into()),
            relative_ns: Some(recorder.elapsed_ns()),
        },
    );
}

async fn pipe<R: AsyncRead + Unpin>(
    reader: R,
    app: tauri::AppHandle,
    version: String,
    kind: &'static str,
    recorder: Option<session::StartupSession>,
) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(recorder) = &recorder {
            recorder.output(kind, &line).await;
            emit_timed(&app, &version, kind, line, None, recorder);
        } else {
            emit(&app, &version, kind, line, None);
        }
    }
}

struct RunningGuard<'a>(&'a AtomicBool);
impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

async fn run_java(
    app: &tauri::AppHandle,
    version: &str,
    java: &str,
    cwd: &std::path::Path,
    args: &[String],
    mut metrics: session::LaunchMetrics,
    diagnostic: bool,
    experimental_hotset: bool,
    skin: Option<Vec<u8>>,
    player: &str,
) -> Result<(), String> {
    let mut launch_args = args.to_vec();
    let skin_server = if let Some(bytes) = skin {
        if let Err(error) = skins::stage_for_instance(cwd, player, &bytes).await {
            emit(
                app,
                version,
                "stderr",
                format!("无法同步 CustomSkinLoader 本地皮肤：{error}"),
                None,
            );
        }
        let server = skins::serve(bytes).await?;
        inject_skin_properties(&mut launch_args, player, &server.url)?;
        Some(server)
    } else {
        None
    };
    let jfr_path = if diagnostic {
        let major = java_major(java.trim()).await?;
        if major < 11 {
            return Err("启动诊断需要 Java 11 或更高版本；普通启动不受影响".into());
        }
        let root = cwd.parent().unwrap_or(cwd);
        let directory = root.join(".catcl/startup/recordings");
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|e| e.to_string())?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis();
        let path = directory.join(format!("startup-{stamp}.jfr"));
        launch_args.insert(
            0,
            format!(
                "-XX:StartFlightRecording=filename={},settings=profile,dumponexit=true",
                path.to_string_lossy()
            ),
        );
        Some(path)
    } else {
        None
    };
    let process_start = std::time::Instant::now();
    let mut command = tokio::process::Command::new(java.trim());
    command
        .args(&launch_args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动 Minecraft：{e}"))?;
    let pid = child.id().unwrap_or_default();
    let instance_root = cwd.parent().unwrap_or(cwd);
    let hotset_active = experimental_hotset && hotset::available(instance_root);
    metrics.hotset_status = if hotset_active {
        "active"
    } else if experimental_hotset {
        "auto-disabled"
    } else {
        "disabled"
    }
    .into();
    if hotset_active {
        hotset::start(instance_root.to_owned(), metrics.instance_generation, 256);
    }
    let recorder =
        session::StartupSession::create(cwd, version, pid, process_start, metrics, jfr_path)
            .await
            .ok();
    if let Some(recorder) = &recorder {
        recorder
            .event("Process Spawn", "process", "Java process created")
            .await;
        emit_timed(
            app,
            version,
            "started",
            format!("Minecraft {version} 已启动（PID {pid}）"),
            None,
            recorder,
        );
    } else {
        emit(
            app,
            version,
            "started",
            format!("Minecraft {version} 已启动（PID {pid}）"),
            None,
        );
    }
    let stdout_task = child.stdout.take().map(|stdout| {
        tokio::spawn(pipe(
            stdout,
            app.clone(),
            version.to_owned(),
            "stdout",
            recorder.clone(),
        ))
    });
    let stderr_task = child.stderr.take().map(|stderr| {
        tokio::spawn(pipe(
            stderr,
            app.clone(),
            version.to_owned(),
            "stderr",
            recorder.clone(),
        ))
    });
    let status = child.wait().await.map_err(|e| e.to_string())?;
    drop(skin_server);
    if let Some(task) = stdout_task {
        let _ = task.await;
    }
    if let Some(task) = stderr_task {
        let _ = task.await;
    }
    let exit_message = format!(
        "Minecraft 已退出，代码 {}",
        status
            .code()
            .map_or_else(|| "未知".into(), |code| code.to_string())
    );
    if let Some(recorder) = &recorder {
        recorder.finish(status.code()).await;
        emit_timed(
            app,
            version,
            "exited",
            exit_message,
            status.code(),
            recorder,
        );
    } else {
        emit(app, version, "exited", exit_message, status.code());
    }
    Ok(())
}

fn inject_skin_properties(args: &mut Vec<String>, player: &str, url: &str) -> Result<(), String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let profile_id = offline_uuid(player).replace('-', "");
    let texture = serde_json::json!({
        "timestamp": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_millis(),
        "profileId": profile_id,
        "profileName": player,
        "textures": { "SKIN": { "url": url } }
    });
    let encoded = STANDARD.encode(serde_json::to_vec(&texture).map_err(|e| e.to_string())?);
    // Modern authlib deserializes userProperties as Map<String, List<String>>.
    // Passing profile-property objects here makes Gson call getAsString on a
    // JsonObject and crashes the game before its main menu is created.
    let properties = serde_json::json!({ "textures": [encoded] }).to_string();
    if let Some(index) = args.iter().position(|value| value == "--userProperties") {
        if let Some(value) = args.get_mut(index + 1) {
            *value = properties;
        } else {
            args.push(properties);
        }
    } else {
        args.push("--userProperties".into());
        args.push(properties);
    }
    Ok(())
}

#[tauri::command]
pub async fn launch_game(
    app: tauri::AppHandle,
    state: State<'_, GameState>,
    request: LaunchRequest,
) -> Result<(), String> {
    let LaunchRequest {
        base_version,
        launch_version,
        instance_path,
        java_path,
        memory_mb,
        mut offline_name,
        account_mode,
        diagnostic,
        experimental_hotset,
    } = request;
    if !valid_version(&base_version) || !valid_version(&launch_version) {
        return Err("无效版本 ID".into());
    }
    if !(1024..=32768).contains(&memory_mb) {
        return Err("内存必须在 1024–32768 MB 之间".into());
    }
    validate_offline_name(&offline_name)?;
    let account = accounts::session(&app, &account_mode).await?;
    if account_mode != "offline" && account.is_none() {
        return Err("所选账户尚未登录，请先在设置中登录".into());
    }
    if let Some(account) = &account {
        offline_name = account.player_name.clone();
    }
    let root = PathBuf::from(instance_path);
    // A plain offline account intentionally starts with the game's default skin.
    let skin = if account_mode == "offline" { None } else { skins::skin_bytes(&app, &offline_name).await };
    if !root.is_absolute() {
        return Err("游戏目录必须是绝对路径".into());
    }
    if state.running.swap(true, Ordering::AcqRel) {
        return Err("Minecraft 已在运行".into());
    }
    let guard = RunningGuard(&state.running);
    let version_dir = safe_join(&root, &format!("versions/{base_version}"))?;
    if !version_dir.join(".verdant-installed").is_file() {
        return Err("该版本尚未完整下载，请先执行下载 / 修复".into());
    }
    let mut trace = trace::LaunchTrace::new();
    let (cache_root, cache_base, cache_launch, cache_java, cache_player) = (
        root.clone(),
        base_version.clone(),
        launch_version.clone(),
        java_path.clone(),
        format!("{}:{}", offline_name, account.as_ref().map(|value| format!("{:x}", md5::compute(&value.access_token))).unwrap_or_default()),
    );
    let cache_check = tokio::task::spawn_blocking(move || {
        cache::check(
            &cache_root,
            &cache_base,
            &cache_launch,
            &cache_java,
            memory_mb,
            &cache_player,
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    trace.mark("实例指纹");
    if let Some(snapshot) = cache_check.snapshot.clone() {
        for stage in [
            "版本解析",
            "Java 探测",
            "Mod 扫描",
            "libraries",
            "assets",
            "natives",
            "classpath",
            "参数构建",
        ] {
            trace.skip(stage);
        }
        emit(&app, &launch_version, "stdout", trace.report(1.0), None);
        return run_java(
            &app,
            &launch_version,
            &snapshot.java_path,
            &PathBuf::from(snapshot.working_directory),
            &snapshot.arguments,
            session::LaunchMetrics {
                ccl_preparation_ns: trace.elapsed_ns(),
                cache_hit_rate: 1.0,
                instance_generation: snapshot.generation,
                java: snapshot.java_path.clone(),
                hotset_status: String::new(),
            },
            diagnostic,
            experimental_hotset,
            skin,
            &offline_name,
        )
        .await;
    }
    let metadata = load_metadata(&root, &base_version, &launch_version).await?;
    trace.mark("版本解析");
    let required_java = metadata["javaVersion"]["majorVersion"]
        .as_u64()
        .unwrap_or(8) as u32;
    let actual_java = java_major(java_path.trim()).await?;
    trace.mark("Java 探测");
    if actual_java != required_java {
        return Err(format!(
            "Minecraft {base_version} 需要 Java {required_java}，当前 Java 为 {actual_java}"
        ));
    }
    let client = version_dir.join(format!("{base_version}.jar"));
    if !client.is_file() {
        return Err("客户端 JAR 缺失，请先执行下载 / 修复".into());
    }
    let mut classpath = Vec::new();
    let mut classpath_seen = std::collections::HashSet::new();
    for library in metadata["libraries"]
        .as_array()
        .ok_or("版本缺少 libraries")?
    {
        if allowed(library) {
            let path = library["downloads"]["artifact"]["path"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| library["name"].as_str().and_then(maven_library_path));
            if let Some(path) = path {
                let path = safe_join(&root, &format!("libraries/{path}"))?;
                if classpath_seen.insert(path.clone()) {
                    classpath.push(path);
                }
            }
        }
    }
    classpath.push(client);
    let classpath_paths = classpath.clone();
    let classpath_entries = classpath.len();
    if classpath.iter().any(|path| !path.is_file()) {
        return Err("依赖文件不完整，请先执行下载 / 修复".into());
    }
    trace.mark("libraries 检查");
    let instance = safe_join(&root, "game")?;
    tokio::fs::create_dir_all(&instance)
        .await
        .map_err(|e| e.to_string())?;
    let assets = safe_join(&root, "assets")?;
    let libraries = safe_join(&root, "libraries")?;
    let natives = version_dir.join("natives");
    let classpath = std::env::join_paths(classpath)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .into_owned();
    trace.mark("classpath");
    let account_uuid = account.as_ref().map(|value| value.uuid.replace('-', "")).unwrap_or_else(|| offline_uuid(&offline_name));
    let account_token = account.as_ref().map(|value| value.access_token.clone()).unwrap_or_else(|| "0".into());
    let account_xuid = account.as_ref().map(|value| value.xuid.clone()).unwrap_or_default();
    let account_type = if account_mode == "offline" { "legacy" } else { "mojang" };
    let variables = HashMap::from([
        ("natives_directory", natives.to_string_lossy().into_owned()),
        ("launcher_name", "CatCL".into()),
        ("launcher_version", env!("CARGO_PKG_VERSION").into()),
        ("classpath", classpath),
        ("classpath_separator", ";".into()),
        (
            "library_directory",
            libraries.to_string_lossy().into_owned(),
        ),
        ("auth_player_name", offline_name.clone()),
        ("version_name", launch_version.clone()),
        ("game_directory", instance.to_string_lossy().into_owned()),
        ("assets_root", assets.to_string_lossy().into_owned()),
        (
            "assets_index_name",
            metadata["assetIndex"]["id"]
                .as_str()
                .ok_or("缺少资源索引")?
                .into(),
        ),
        ("auth_uuid", account_uuid),
        ("auth_access_token", account_token),
        ("clientid", String::new()),
        ("auth_xuid", account_xuid),
        ("user_type", account_type.into()),
        (
            "version_type",
            metadata["type"].as_str().unwrap_or("release").into(),
        ),
        ("user_properties", "{}".into()),
        ("resolution_width", "854".into()),
        ("resolution_height", "480".into()),
    ]);
    let mut args = vec![
        format!("-Xmx{memory_mb}M"),
        // Pre-allocate the heap to avoid repeated resizing during modded startup.
        format!("-Xms{}M", (memory_mb as usize / 2).clamp(1024, 8192)),
    ];
    append_arguments(&mut args, &metadata["arguments"]["jvm"], &variables, false)?;
    if account_mode == "littleskin" {
        let (jar, prefetched) = accounts::ensure_authlib_injector(&app).await?;
        args.push(format!("-javaagent:{}={}", jar.to_string_lossy(), accounts::LITTLE_SKIN_API));
        args.push(format!("-Dauthlibinjector.yggdrasil.prefetched={prefetched}"));
    }
    add_optimized_jvm_args(&mut args, actual_java, base_version != launch_version);
    if base_version != launch_version {
        args.push("-XX:ReservedCodeCacheSize=512M".into());
        args.push("-XX:+UseCompressedOops".into());
    }
    if let Some(logging) = metadata["logging"]["client"].as_object() {
        let argument = logging["argument"].as_str().ok_or("日志参数格式错误")?;
        let path = safe_join(
            &root,
            &format!(
                "assets/log_configs/{}",
                logging["file"]["id"].as_str().ok_or("日志配置缺少 ID")?
            ),
        )?;
        args.push(argument.replace("${path}", &path.to_string_lossy()));
    }
    let main_class = metadata["mainClass"]
        .as_str()
        .ok_or("版本缺少 mainClass")?
        .to_owned();
    args.push(main_class.clone());
    append_arguments(&mut args, &metadata["arguments"]["game"], &variables, false)?;
    trace.mark("参数构建");
    let snapshot = profile::LaunchSnapshot {
        schema: cache::CACHE_SCHEMA,
        generation: 0,
        java_path: java_path.trim().into(),
        working_directory: instance.to_string_lossy().into_owned(),
        arguments: args.clone(),
        classpath_entries,
        main_class,
    };
    let cache_rate = cache_check.hit_rate;
    let cache_result =
        tokio::task::spawn_blocking(move || cache::commit(cache_check, snapshot, &classpath_paths))
            .await;
    let mut generation = 0;
    match cache_result {
        Ok(Ok(stats)) => {
            generation = stats.generation;
            trace.record("Mod 扫描", stats.mods);
            trace.record("libraries 缓存", stats.libraries);
            trace.record("assets 缓存", stats.assets);
            trace.record("natives 缓存", stats.natives);
        }
        Ok(Err(error)) => emit(
            &app,
            &launch_version,
            "stderr",
            format!("启动缓存写入失败，已继续启动：{error}"),
            None,
        ),
        Err(error) => emit(
            &app,
            &launch_version,
            "stderr",
            format!("启动缓存任务异常，已继续启动：{error}"),
            None,
        ),
    }
    trace.mark("L2 缓存");
    emit(
        &app,
        &launch_version,
        "stdout",
        trace.report(cache_rate),
        None,
    );
    if base_version != launch_version {
        emit(
            &app,
            &launch_version,
            "stdout",
            format!(
                "已启用大型整合包启动优化 · {} 个类路径项目 · G1GC",
                classpath_entries
            ),
            None,
        );
    }
    run_java(
        &app,
        &launch_version,
        java_path.trim(),
        &instance,
        &args,
        session::LaunchMetrics {
            ccl_preparation_ns: trace.elapsed_ns(),
            cache_hit_rate: cache_rate,
            instance_generation: generation,
            java: java_path.trim().into(),
            hotset_status: String::new(),
        },
        diagnostic,
        experimental_hotset,
        skin,
        &offline_name,
    )
    .await?;
    drop(guard);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expands_and_rejects_unknown_placeholders() {
        let variables = HashMap::from([("name", "Player".into())]);
        assert_eq!(
            expand("--username=${name}", &variables).unwrap(),
            "--username=Player"
        );
        assert!(expand("${unknown}", &variables).is_err());
    }
    #[test]
    fn demo_feature_rules_are_applied() {
        let demo = serde_json::json!({"rules":[{"action":"allow","features":{"is_demo_user":true}}],"value":"--demo"});
        assert!(argument_allowed(&demo, true));
        assert!(!argument_allowed(&demo, false));
    }
    #[test]
    fn creates_java_compatible_offline_uuid() {
        assert_eq!(
            offline_uuid("Notch"),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
        assert_eq!(offline_uuid("Player"), offline_uuid("Player"));
        assert_ne!(offline_uuid("Player"), offline_uuid("player"));
    }
    #[test]
    fn injects_offline_skin_texture_without_rebuilding_snapshot() {
        let mut args = vec!["--userProperties".into(), "{}".into()];
        inject_skin_properties(&mut args, "Player", "http://127.0.0.1:1234/skin.png").unwrap();
        let properties: Value = serde_json::from_str(&args[1]).unwrap();
        let encoded = properties["textures"][0].as_str().unwrap();
        use base64::{engine::general_purpose::STANDARD, Engine};
        let texture: Value = serde_json::from_slice(&STANDARD.decode(encoded).unwrap()).unwrap();
        assert_eq!(texture["profileName"], "Player");
        assert_eq!(
            texture["textures"]["SKIN"]["url"],
            "http://127.0.0.1:1234/skin.png"
        );
    }
    #[test]
    fn appends_skin_properties_for_modern_versions_that_omit_the_argument() {
        let mut args = vec![
            "net.minecraft.client.main.Main".into(),
            "--username".into(),
            "Player".into(),
        ];
        inject_skin_properties(&mut args, "Player", "http://127.0.0.1/skin.png").unwrap();
        let index = args
            .iter()
            .position(|value| value == "--userProperties")
            .unwrap();
        assert!(serde_json::from_str::<Value>(&args[index + 1]).is_ok());
    }
    #[test]
    fn resolves_maven_library_coordinates() {
        assert_eq!(
            maven_library_path("net.fabricmc:fabric-loader:0.16.0").as_deref(),
            Some("net/fabricmc/fabric-loader/0.16.0/fabric-loader-0.16.0.jar")
        );
        assert_eq!(
            maven_library_path("group:name:1.0:natives-windows").as_deref(),
            Some("group/name/1.0/name-1.0-natives-windows.jar")
        );
    }
    #[test]
    fn assembles_scalar_array_and_demo_arguments() {
        let variables = HashMap::from([("game_directory", "D:/Games/Test".into())]);
        let values = serde_json::json!([
            "--gameDir", "${game_directory}",
            {"rules":[{"action":"allow","features":{"is_demo_user":true}}],"value":["--demo","--demoMode"]},
            {"rules":[{"action":"allow","features":{"has_custom_resolution":true}}],"value":"--width"}
        ]);
        let mut args = Vec::new();
        append_arguments(&mut args, &values, &variables, true).unwrap();
        assert_eq!(args, ["--gameDir", "D:/Games/Test", "--demo", "--demoMode"]);
    }

    #[test]
    fn child_libraries_replace_base_versions_without_duplicates() {
        let mut parent = serde_json::json!({"libraries":[
            {"name":"example:core:1.0"},
            {"name":"example:keep:1.0"}
        ]});
        let child = serde_json::json!({"libraries":[
            {"name":"example:core:2.0"},
            {"name":"example:new:1.0"}
        ]});
        merge_libraries(&mut parent, &child).unwrap();
        let names: Vec<_> = parent["libraries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|library| library["name"].as_str())
            .collect();
        assert_eq!(
            names,
            ["example:core:2.0", "example:keep:1.0", "example:new:1.0"]
        );
    }

    #[test]
    fn optimized_flags_do_not_override_loader_choices() {
        let mut args = vec!["-XX:-UseG1GC".into()];
        add_optimized_jvm_args(&mut args, 21, true);
        assert_eq!(args.iter().filter(|arg| arg.contains("UseG1GC")).count(), 1);
        assert!(args.iter().any(|arg| arg == "-XX:+ParallelRefProcEnabled"));
        assert!(args.iter().any(|arg| arg == "-XX:+UseStringDeduplication"));
    }
}
