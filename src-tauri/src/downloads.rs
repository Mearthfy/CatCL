use futures_util::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{Emitter, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Default)]
pub struct DownloadState {
    active: Mutex<Option<CancellationToken>>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameVersion {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    release_time: String,
    #[serde(skip_serializing)]
    url: String,
    #[serde(skip_serializing)]
    sha1: String,
}

#[derive(Deserialize)]
struct Manifest {
    versions: Vec<GameVersion>,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(300))
        .user_agent("CatCL/0.1.0")
        .build()
        .map_err(|e| e.to_string())
}

async fn manifest(client: &reqwest::Client) -> Result<Manifest, String> {
    client
        .get(MANIFEST)
        .send()
        .await
        .map_err(|e| format!("无法连接 Mojang：{e}"))?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

fn supported(version: &GameVersion) -> bool {
    // Old versions use legacy arguments and asset layouts; keep this milestone scoped.
    (version.kind == "release" || version.kind == "snapshot")
        && version.release_time.as_str() >= "2018-07-18"
}

#[tauri::command]
pub async fn list_versions() -> Result<Vec<GameVersion>, String> {
    Ok(manifest(&client()?)
        .await?
        .versions
        .into_iter()
        .filter(supported)
        .collect())
}

pub fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.contains(':')
        || relative.contains('\\')
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err(format!("无效的资源路径：{relative}"));
    }
    Ok(root.join(path))
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value[name]
        .as_str()
        .ok_or_else(|| format!("版本信息缺少字段：{name}"))
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 40 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}

#[derive(Clone)]
struct FileJob {
    url: String,
    path: PathBuf,
    sha1: String,
}

impl FileJob {
    fn from(value: &Value, path: PathBuf) -> Result<Self, String> {
        let sha1 = field(value, "sha1")?;
        if !valid_hash(sha1) {
            return Err("资源 SHA-1 格式无效".into());
        }
        Ok(Self {
            url: field(value, "url")?.into(),
            path,
            sha1: sha1.into(),
        })
    }
}

async fn file_valid(path: &Path, expected: &str) -> Result<bool, String> {
    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    let mut digest = Sha1::new();
    let mut buffer = vec![0; 65536];
    loop {
        let count = file.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()).eq_ignore_ascii_case(expected))
}

// Removes partial downloads on cancellation, including when a concurrent future is dropped.
struct PartialFile(PathBuf);
impl Drop for PartialFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn download(
    client: &reqwest::Client,
    job: &FileJob,
    token: &CancellationToken,
) -> Result<(), String> {
    tokio::select! {
        biased;
        _ = token.cancelled() => Err("下载已取消".into()),
        result = download_file(client, job) => result,
    }
}

async fn download_file(client: &reqwest::Client, job: &FileJob) -> Result<(), String> {
    if file_valid(&job.path, &job.sha1).await? {
        return Ok(());
    }
    let parent = job.path.parent().ok_or("无效目标路径")?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| e.to_string())?;
    let temp = PartialFile(job.path.with_extension(format!(
        "{}.part",
        job.path.extension().unwrap_or_default().to_string_lossy()
    )));
    {
        let response = client
            .get(&job.url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        let mut chunks = response.bytes_stream();
        let mut output = tokio::fs::File::create(&temp.0)
            .await
            .map_err(|e| e.to_string())?;
        let mut digest = Sha1::new();
        while let Some(chunk) = chunks.next().await {
            let bytes = chunk.map_err(|e| e.to_string())?;
            digest.update(&bytes);
            output.write_all(&bytes).await.map_err(|e| e.to_string())?;
        }
        output.flush().await.map_err(|e| e.to_string())?;
        if !format!("{:x}", digest.finalize()).eq_ignore_ascii_case(&job.sha1) {
            return Err(format!("文件校验失败：{}", job.path.display()));
        }
    }
    tokio::fs::rename(&temp.0, &job.path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn allowed(library: &Value) -> bool {
    let Some(rules) = library["rules"].as_array() else {
        return true;
    };
    let mut result = false;
    for rule in rules {
        if let Some(os) = rule.get("os") {
            if os["name"].as_str().is_some_and(|n| n != "windows") {
                continue;
            }
            if os["arch"]
                .as_str()
                .is_some_and(|n| n != "x86_64" && n != "amd64")
            {
                continue;
            }
            // OS version rules currently occur on launch arguments, not supported library rules.
            if os.get("version").is_some() {
                continue;
            }
        }
        if rule["features"]
            .as_object()
            .is_some_and(|f| f.values().any(|v| v.as_bool() != Some(false)))
        {
            continue;
        }
        result = rule["action"] == "allow";
    }
    result
}

#[derive(Clone, Serialize)]
struct Progress {
    version: String,
    completed: usize,
    total: usize,
    message: String,
}

fn emit(app: &tauri::AppHandle, version: &str, completed: usize, total: usize, message: String) {
    let _ = app.emit(
        "install-progress",
        Progress {
            version: version.into(),
            completed,
            total,
            message,
        },
    );
}

async fn install(
    app: &tauri::AppHandle,
    version: &str,
    root: &Path,
    token: &CancellationToken,
) -> Result<(), String> {
    let http = client()?;
    let manifest = tokio::select! {
        biased;
        _ = token.cancelled() => return Err("下载已取消".into()),
        result = manifest(&http) => result?,
    };
    let entry = manifest
        .versions
        .into_iter()
        .find(|v| v.id == version && supported(v))
        .ok_or("官方列表中没有此版本或暂不支持此版本")?;
    let metadata_path = safe_join(root, &format!("versions/{version}/{version}.json"))?;
    let metadata = FileJob {
        url: entry.url,
        path: metadata_path.clone(),
        sha1: entry.sha1,
    };
    download(&http, &metadata, token).await?;
    let json: Value = serde_json::from_slice(
        &tokio::fs::read(&metadata_path)
            .await
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if !json["arguments"].is_object() {
        return Err("暂不支持旧版启动格式".into());
    }
    let mut jobs = vec![FileJob::from(
        &json["downloads"]["client"],
        safe_join(root, &format!("versions/{version}/{version}.jar"))?,
    )?];
    let mut natives = Vec::new();
    for library in json["libraries"].as_array().ok_or("版本缺少 libraries")? {
        if !allowed(library) {
            continue;
        }
        if let Some(artifact) = library["downloads"].get("artifact") {
            jobs.push(FileJob::from(
                artifact,
                safe_join(root, &format!("libraries/{}", field(artifact, "path")?))?,
            )?);
        }
        if let Some(classifier) = library["natives"]["windows"].as_str() {
            let native = &library["downloads"]["classifiers"][classifier.replace("${arch}", "64")];
            let path = safe_join(root, &format!("libraries/{}", field(native, "path")?))?;
            jobs.push(FileJob::from(native, path.clone())?);
            natives.push(path);
        }
    }
    let index = &json["assetIndex"];
    let index_path = safe_join(
        root,
        &format!("assets/indexes/{}.json", field(index, "id")?),
    )?;
    download(&http, &FileJob::from(index, index_path.clone())?, token).await?;
    let assets: Value = serde_json::from_slice(
        &tokio::fs::read(index_path)
            .await
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    for asset in assets["objects"]
        .as_object()
        .ok_or("资源索引缺少 objects")?
        .values()
    {
        let hash = field(asset, "hash")?;
        if !valid_hash(hash) {
            return Err("资源哈希无效".into());
        }
        let suffix = format!("{}/{hash}", &hash[..2]);
        jobs.push(FileJob {
            url: format!("https://resources.download.minecraft.net/{suffix}"),
            path: safe_join(root, &format!("assets/objects/{suffix}"))?,
            sha1: hash.into(),
        });
    }
    if let Some(log_file) = json["logging"]["client"].get("file") {
        jobs.push(FileJob::from(
            log_file,
            safe_join(
                root,
                &format!("assets/log_configs/{}", field(log_file, "id")?),
            )?,
        )?);
    }
    let mut seen = HashSet::new();
    jobs.retain(|j| seen.insert(j.path.clone()));
    let total = jobs.len();
    emit(app, version, 0, total, "下载并校验游戏文件…".into());
    let done = Arc::new(AtomicUsize::new(0));
    stream::iter(jobs.into_iter().map(|job| {
        let http = &http;
        let done = done.clone();
        async move {
            let mut last_error = String::new();
            for attempt in 0..3 {
                if token.is_cancelled() {
                    return Err("下载已取消".into());
                }
                match download(http, &job, token).await {
                    Ok(()) => {
                        let completed = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if completed.is_multiple_of(20) || completed == total {
                            emit(
                                app,
                                version,
                                completed,
                                total,
                                format!("已准备 {completed} / {total} 个文件"),
                            );
                        }
                        return Ok::<(), String>(());
                    }
                    Err(error) => {
                        last_error = error;
                        if attempt < 2 {
                            tokio::time::sleep(Duration::from_millis(500 * (attempt + 1))).await;
                        }
                    }
                }
            }
            Err(format!("{}：{last_error}", job.path.display()))
        }
    }))
    .buffer_unordered(8)
    .try_collect::<Vec<_>>()
    .await?;

    let native_root = safe_join(root, &format!("versions/{version}/natives"))?;
    emit(app, version, total, total, "整理本地运行库…".into());
    // Await this blocking task even if cancelled, so no detached extraction can race a new install.
    let extract_token = token.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        std::fs::create_dir_all(&native_root).map_err(|e| e.to_string())?;
        for archive in natives {
            let mut zip =
                zip::ZipArchive::new(std::fs::File::open(archive).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            for index in 0..zip.len() {
                if extract_token.is_cancelled() {
                    return Err("下载已取消".into());
                }
                let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
                if !entry.name().to_ascii_lowercase().ends_with(".dll") {
                    continue;
                }
                let target = safe_join(&native_root, entry.name())?;
                std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
                let mut output = std::fs::File::create(target).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    if token.is_cancelled() {
        return Err("下载已取消".into());
    }
    let marker = safe_join(root, &format!("versions/{version}/.verdant-installed"))?;
    tokio::fs::write(marker, b"1")
        .await
        .map_err(|e| e.to_string())?;
    emit(app, version, total, total, "游戏文件已准备好".into());
    Ok(())
}

#[tauri::command]
pub async fn install_version(
    app: tauri::AppHandle,
    state: State<'_, DownloadState>,
    version: String,
    game_dir: String,
) -> Result<(), String> {
    if cfg!(not(all(windows, target_arch = "x86_64"))) {
        return Err("当前下载器仅支持 Windows x64".into());
    }
    if version.is_empty() || version.contains(['/', '\\', ':']) || version == "." || version == ".."
    {
        return Err("无效版本 ID".into());
    }
    let root = PathBuf::from(game_dir);
    if !root.is_absolute() {
        return Err("请先设置绝对游戏目录".into());
    }
    let token = CancellationToken::new();
    {
        let mut active = state.active.lock().map_err(|e| e.to_string())?;
        if active.is_some() {
            return Err("已有下载任务正在运行".into());
        }
        *active = Some(token.clone());
    }
    let result = install(&app, &version, &root, &token).await;
    *state.active.lock().map_err(|e| e.to_string())? = None;
    result
}

#[tauri::command]
pub fn cancel_install(state: State<'_, DownloadState>) -> Result<(), String> {
    if let Some(token) = state.active.lock().map_err(|e| e.to_string())?.as_ref() {
        token.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn installed_versions(game_dir: String) -> Result<Vec<String>, String> {
    if !Path::new(&game_dir).is_absolute() {
        return Err("游戏目录必须是绝对路径".into());
    }
    let mut entries = match tokio::fs::read_dir(Path::new(&game_dir).join("versions")).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut versions = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        if entry.path().join(".verdant-installed").is_file() {
            versions.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    versions.sort();
    Ok(versions)
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn read_request(socket: &mut tokio::net::TcpStream) {
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(socket.read_u8().await.unwrap());
            assert!(bytes.len() < 8192);
        }
    }
    #[tokio::test]
    #[ignore = "Requires access to Mojang's production service"]
    async fn official_manifest_and_metadata_smoke() {
        let http = client().unwrap();
        let versions = manifest(&http).await.unwrap();
        let version = versions.versions.iter().find(|v| v.id == "1.21.1").unwrap();
        assert!(supported(version));
        let bytes = http
            .get(&version.url)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(format!("{:x}", Sha1::digest(&bytes)), version.sha1);
        let metadata: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(metadata["arguments"]["jvm"].is_array());
        assert!(metadata["libraries"]
            .as_array()
            .unwrap()
            .iter()
            .any(allowed));
        assert!(metadata["downloads"]["client"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://"));
    }

    #[test]
    fn rejects_path_traversal() {
        let root = std::env::temp_dir();
        for path in [
            "../outside",
            "/absolute",
            "C:/windows",
            "a/../../b",
            "a\\..\\b",
            "",
        ] {
            assert!(safe_join(&root, path).is_err(), "{path}");
        }
        assert_eq!(
            safe_join(&root, "assets/objects/ab/hash").unwrap(),
            root.join("assets/objects/ab/hash")
        );
    }
    #[test]
    fn applies_os_rules_in_order() {
        assert!(allowed(&serde_json::json!({})));
        assert!(!allowed(
            &serde_json::json!({"rules": [{"action":"allow", "os":{"name":"osx"}}]})
        ));
        assert!(allowed(
            &serde_json::json!({"rules": [{"action":"allow"}, {"action":"disallow", "os":{"name":"osx"}}]})
        ));
        assert!(!allowed(
            &serde_json::json!({"rules": [{"action":"allow"}, {"action":"disallow", "os":{"name":"windows"}}]})
        ));
    }
    #[tokio::test]
    async fn detects_corrupt_download() {
        let path = std::env::temp_dir().join(format!("verdant-hash-test-{}", std::process::id()));
        tokio::fs::write(&path, b"abc").await.unwrap();
        assert!(
            file_valid(&path, "a9993e364706816aba3e25717850c26c9cd0d89d")
                .await
                .unwrap()
        );
        assert!(
            !file_valid(&path, "0000000000000000000000000000000000000000")
                .await
                .unwrap()
        );
        tokio::fs::remove_file(path).await.unwrap();
    }

    #[tokio::test]
    async fn streams_and_verifies_download_then_reuses_cached_file() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc")
                .await
                .unwrap();
        });
        let path = std::env::temp_dir().join(format!("verdant-stream-test-{}", std::process::id()));
        let job = FileJob {
            url: format!("http://{address}"),
            path: path.clone(),
            sha1: "a9993e364706816aba3e25717850c26c9cd0d89d".into(),
        };
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        download(&client, &job, &token).await.unwrap();
        server.await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"abc");
        // Server is gone; a second call must use the verified local file.
        download(&client, &job, &token).await.unwrap();
        tokio::fs::remove_file(path).await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_stops_an_inflight_download_without_publishing_file() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let token = CancellationToken::new();
        let cancel = token.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9999\r\n\r\na")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel.cancel();
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let path = std::env::temp_dir().join(format!("verdant-cancel-test-{}", std::process::id()));
        let job = FileJob {
            url: format!("http://{address}"),
            path: path.clone(),
            sha1: "0000000000000000000000000000000000000000".into(),
        };
        let result = download(&reqwest::Client::new(), &job, &token).await;
        server.await.unwrap();
        assert_eq!(result.unwrap_err(), "下载已取消");
        assert!(!path.exists());
        assert!(!path.with_extension(".part").exists());
    }
}
